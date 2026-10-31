//! Non-mesh catalog layers: stored feature tables (`Attributes`) with their packed geometry, and
//! the 2D cell centre points.
//!
//! Every feature layer has the same shape: `source_row` (Int64, the zero-based row of the stored
//! `Attributes` table, or of `Info` when the family has no `Attributes`), then every stored
//! `Attributes` column under its stored name and in stored order, then `geometry` (WKB). No
//! column is renamed, derived, joined from another table or converted from text; consumers
//! rename through [`super::FIELD_ALIASES`].
//!
//! Geometry follows the packed `Info` / `Parts` / `Points` triple exactly:
//!
//! - lines: one part is a LineString, several parts a MultiLineString (as ras-commander's
//!   `HdfBase.get_polylines_from_parts`); every part needs two points.
//! - polygons: one part is a Polygon, closed when the stored ring is open (as Shapely closes
//!   it); it needs three points. Several parts are refused: ras-commander assembles them
//!   differently per family (holes by containment, or a MultiPolygon), and that is not
//!   reproduced here.
//! - a feature with no points (and no part, or only empty parts) has a null geometry, as
//!   ras-commander gives for structures without a drawn centerline.
//! - coordinates must be stored as two columns (X, Y); other widths are refused.
use super::{geometry_field, refuse, wkb_line, wkb_polygon, ArrowLayer, LayerOptions};
use crate::hdf::metadata::{CompoundTable, MetadataValue};
use crate::hdf::types::AttributeType;
use crate::hdf::{NumericArray, NumericValues, ReadOnlyHdfSession};
use crate::{Capability, Error, ErrorKind, Reported, Result, UnsupportedCode};
use arrow_array::builder::{BinaryBuilder, StringBuilder};
use arrow_array::{ArrayRef, Float32Array, Float64Array, Int64Array, UInt64Array};
use arrow_schema::{DataType, Field};
use std::ops::Range;
use std::sync::Arc;

/// Geometry type of a feature layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Shape {
    Line,
    Polygon,
    Point,
}

/// One stored family, normalised: attributes, XY points and the point ranges of each part.
struct Packed {
    table: Option<CompoundTable>,
    points: Vec<[f64; 2]>,
    /// Per feature: its whole point range and its parts (absolute ranges into `points`).
    features: Vec<(Range<usize>, Vec<Range<usize>>)>,
}

fn take<T>(layer: &str, r: Reported<T>) -> Result<T> {
    match r.capability().clone() {
        Capability::Supported => r
            .into_value()
            .ok_or_else(|| Error::new(ErrorKind::Internal, "supported value missing")),
        Capability::Unavailable { reason } => Err(Error::new(
            ErrorKind::NotFound,
            format!("{layer}: not stored in this file ({reason})"),
        )),
        Capability::Unsupported { code, reason } => Err(refuse(code, format!("{layer}: {reason}"))),
        Capability::Corrupt { reason } => {
            Err(Error::new(ErrorKind::Corrupt, format!("{layer}: {reason}")))
        }
        Capability::Delegated { reason } => Err(refuse(
            UnsupportedCode::NotImplemented,
            format!("{layer}: {reason}"),
        )),
    }
}

fn xy(layer: &str, a: &NumericArray) -> Result<Vec<[f64; 2]>> {
    if a.shape.len() != 2 || a.shape[1] != 2 {
        return Err(refuse(
            UnsupportedCode::Schema,
            format!(
                "{layer}: points stored with shape {:?}; only (n, 2) is read",
                a.shape
            ),
        ));
    }
    let flat: Vec<f64> = match &a.values {
        NumericValues::F64(v) => v.clone(),
        NumericValues::F32(v) => v.iter().map(|&x| f64::from(x)).collect(),
        _ => {
            return Err(refuse(
                UnsupportedCode::Encoding,
                format!("{layer}: coordinates are not f32/f64"),
            ))
        }
    };
    Ok(flat.as_chunks::<2>().0.to_vec())
}

fn usize_range(r: &Range<u64>) -> Result<Range<usize>> {
    let c = |v: u64| {
        usize::try_from(v).map_err(|_| refuse(UnsupportedCode::ResourceLimit, "point index"))
    };
    Ok(c(r.start)?..c(r.end)?)
}

/// Arrow fields and arrays of a stored compound table, under the stored names: text as Utf8,
/// signed integers as Int64, unsigned as UInt64, floats in their stored width (NaN as null).
/// Bitfields, unproven or damaged columns are refused.
fn table_columns(
    layer: &str,
    table: &CompoundTable,
    rows: usize,
) -> Result<(Vec<Field>, Vec<ArrayRef>)> {
    let mut fields = Vec::new();
    let mut arrays: Vec<ArrayRef> = Vec::new();
    for c in &table.columns {
        if matches!(c.name.as_str(), "source_row" | "geometry") || c.name.is_empty() {
            return Err(refuse(
                UnsupportedCode::Schema,
                format!("{layer}: stored column name {:?} is reserved", c.name),
            ));
        }
        let (field, array): (Field, ArrayRef) = match &c.value {
            MetadataValue::Strings(v) if v.len() == rows => {
                let mut b = StringBuilder::new();
                v.iter().for_each(|s| b.append_value(s));
                (
                    Field::new(&c.name, DataType::Utf8, false),
                    Arc::new(b.finish()),
                )
            }
            MetadataValue::Signed(v) if v.len() == rows => (
                Field::new(&c.name, DataType::Int64, false),
                Arc::new(Int64Array::from(v.clone())),
            ),
            MetadataValue::Unsigned(v) if v.len() == rows => (
                Field::new(&c.name, DataType::UInt64, false),
                Arc::new(UInt64Array::from(v.clone())),
            ),
            MetadataValue::Float(v) if v.len() == rows => {
                if matches!(c.attribute_type, AttributeType::Float { bytes: 4, .. }) {
                    let a = Float32Array::from(
                        v.iter()
                            .map(|&x| (!x.is_nan()).then_some(x as f32))
                            .collect::<Vec<_>>(),
                    );
                    (Field::new(&c.name, DataType::Float32, true), Arc::new(a))
                } else {
                    let a = Float64Array::from(
                        v.iter()
                            .map(|&x| (!x.is_nan()).then_some(x))
                            .collect::<Vec<_>>(),
                    );
                    (Field::new(&c.name, DataType::Float64, true), Arc::new(a))
                }
            }
            MetadataValue::Unsupported { code, reason } => {
                return Err(refuse(*code, format!("{layer}/{}: {reason}", c.name)))
            }
            MetadataValue::Corrupt { reason } => {
                return Err(Error::new(
                    ErrorKind::Corrupt,
                    format!("{layer}/{}: {reason}", c.name),
                ))
            }
            MetadataValue::Bitfield(_) => {
                return Err(refuse(
                    UnsupportedCode::Encoding,
                    format!("{layer}/{}: bitfield columns are not read as Arrow", c.name),
                ))
            }
            _ => {
                return Err(Error::new(
                    ErrorKind::Corrupt,
                    format!("{layer}/{}: column length differs from the table", c.name),
                ))
            }
        };
        fields.push(field);
        arrays.push(array);
    }
    Ok((fields, arrays))
}

fn geometry_wkb(
    layer: &str,
    row: usize,
    shape: Shape,
    points: &[[f64; 2]],
    whole: &Range<usize>,
    parts: &[Range<usize>],
    out: &mut Vec<u8>,
) -> Result<bool> {
    out.clear();
    let bad = |m: &str| {
        refuse(
            UnsupportedCode::NotImplemented,
            format!("{layer}: row {row}: {m}"),
        )
    };
    if shape == Shape::Point {
        if whole.len() != 1 {
            return Err(bad("a point feature needs exactly one point"));
        }
        let p = points[whole.start];
        out.push(1);
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&p[0].to_le_bytes());
        out.extend_from_slice(&p[1].to_le_bytes());
        return Ok(true);
    }
    if whole.is_empty() && parts.iter().all(|p| p.is_empty()) {
        // HEC-RAS stores a feature without drawn geometry (some lateral structures) as an
        // empty point range, with no part or one empty part.
        return Ok(false);
    }
    if parts.is_empty() {
        return Err(bad("points without parts"));
    }
    match shape {
        Shape::Line => {
            if parts.iter().any(|p| p.len() < 2) {
                return Err(bad("a line part has fewer than two points"));
            }
            if let [part] = parts {
                if part != whole {
                    return Err(bad("the single part does not span the feature's points"));
                }
                wkb_line(out, &points[part.clone()])?;
            } else {
                out.push(1);
                out.extend_from_slice(&5u32.to_le_bytes());
                out.extend_from_slice(
                    &u32::try_from(parts.len())
                        .map_err(|_| refuse(UnsupportedCode::ResourceLimit, "parts"))?
                        .to_le_bytes(),
                );
                let mut one = Vec::new();
                for p in parts {
                    one.clear();
                    wkb_line(&mut one, &points[p.clone()])?;
                    out.extend_from_slice(&one);
                }
            }
        }
        Shape::Polygon => {
            let [part] = parts else {
                return Err(bad(
                    "multi-part polygons are assembled differently per family by ras-commander; not reproduced",
                ));
            };
            if part != whole || part.len() < 3 {
                return Err(bad("a polygon needs one part of at least three points"));
            }
            let ring = &points[part.clone()];
            if ring.first() == ring.last() {
                wkb_polygon(out, ring)?;
            } else {
                let mut closed = ring.to_vec();
                closed.push(ring[0]);
                wkb_polygon(out, &closed)?;
            }
        }
        Shape::Point => unreachable!("handled above"),
    }
    Ok(true)
}

fn build(layer: &str, shape: Shape, packed: Packed, options: &LayerOptions) -> Result<ArrowLayer> {
    let rows = packed.features.len();
    let mut fields = vec![Field::new("source_row", DataType::Int64, false)];
    let mut columns: Vec<ArrayRef> = vec![Arc::new(Int64Array::from(
        (0..rows)
            .map(|r| i64::try_from(r).map_err(|_| refuse(UnsupportedCode::ResourceLimit, "rows")))
            .collect::<Result<Vec<_>>>()?,
    ))];
    if let Some(t) = &packed.table {
        if t.shape != [rows as u64] {
            return Err(Error::new(
                ErrorKind::Corrupt,
                format!("{layer}: Attributes and geometry row counts differ"),
            ));
        }
        let (f, c) = table_columns(layer, t, rows)?;
        fields.extend(f);
        columns.extend(c);
    }
    let mut geoms = BinaryBuilder::new();
    let mut wkb = Vec::new();
    for (row, (whole, parts)) in packed.features.iter().enumerate() {
        if whole.end > packed.points.len() || parts.iter().any(|p| p.end > packed.points.len()) {
            return Err(Error::new(
                ErrorKind::Corrupt,
                format!("{layer}: row {row}: point range outside Points"),
            ));
        }
        if geometry_wkb(layer, row, shape, &packed.points, whole, parts, &mut wkb)? {
            geoms.append_value(&wkb);
        } else {
            geoms.append_null();
        }
    }
    fields.push(geometry_field());
    columns.push(Arc::new(geoms.finish()));
    ArrowLayer::from_columns(
        layer,
        fields,
        columns,
        ("source_row", None),
        options.batch_rows,
    )
}

impl ReadOnlyHdfSession {
    /// A stored feature layer of the canonical catalog by its canonical name: `bc_lines`,
    /// `breaklines`, `refinement_regions`, `reference_lines`, `reference_points`,
    /// `mannings_n_regions`, `cross_sections`, `river_centerlines`, `river_bank_lines`,
    /// `river_edge_lines`, `river_flow_paths`, `structures`, `storage_areas`,
    /// `river_junctions`, `pump_stations`, `pipe_conduits`, `pipe_nodes`. Columns: `source_row`, the stored `Attributes`
    /// columns under their stored names, `geometry` (see the module notes for the geometry
    /// rules and refusals). A family the file does not store is `NotFound`.
    pub fn feature_layer(&self, layer: &str, options: &LayerOptions) -> Result<ArrowLayer> {
        use crate::hdf::lines::GeometryFamily as G;
        use crate::hdf::xs_geometry::OneDLineFamily as O;
        let budget = options.max_bytes;
        let geometry_family = |f: G, shape: Shape| -> Result<ArrowLayer> {
            let d = take(layer, self.geometry_family(f, budget)?)?;
            let packed = Packed {
                points: xy(layer, &d.points)?,
                features: d
                    .features
                    .iter()
                    .map(|f| {
                        Ok((
                            usize_range(&f.point_range)?,
                            f.parts.iter().map(usize_range).collect::<Result<_>>()?,
                        ))
                    })
                    .collect::<Result<_>>()?,
                table: Some(d.attributes.table),
            };
            build(layer, shape, packed, options)
        };
        let one_d = |f: O, shape: Shape| -> Result<ArrowLayer> {
            let d = take(layer, self.one_d_line_family(f, budget)?)?;
            let packed = Packed {
                points: xy(layer, &d.points)?,
                features: d
                    .features
                    .iter()
                    .map(|f| {
                        Ok((
                            usize_range(&f.point_range)?,
                            f.parts.iter().map(usize_range).collect::<Result<_>>()?,
                        ))
                    })
                    .collect::<Result<_>>()?,
                table: d.attributes.map(|a| a.table),
            };
            build(layer, shape, packed, options)
        };
        let points = |d: crate::hdf::xs_geometry::OneDPointData| -> Result<ArrowLayer> {
            let pts = xy(layer, &d.points)?;
            if pts.len() != d.features.len() {
                return Err(Error::new(
                    ErrorKind::Corrupt,
                    format!("{layer}: Points and Attributes row counts differ"),
                ));
            }
            let packed = Packed {
                features: (0..pts.len()).map(|i| (i..i + 1, Vec::new())).collect(),
                points: pts,
                table: Some(d.attributes.table),
            };
            build(layer, Shape::Point, packed, options)
        };
        match layer {
            "bc_lines" => geometry_family(G::BoundaryConditions, Shape::Line),
            "breaklines" => geometry_family(G::Breaklines, Shape::Line),
            "reference_lines" => geometry_family(G::ReferenceLines, Shape::Line),
            "refinement_regions" => geometry_family(G::RefinementRegions, Shape::Polygon),
            "mannings_n_regions" => geometry_family(G::CalibrationRegions, Shape::Polygon),
            "cross_sections" => one_d(O::CrossSectionCutLines, Shape::Line),
            "river_centerlines" => one_d(O::RiverCenterlines, Shape::Line),
            "river_bank_lines" => one_d(O::RiverBankLines, Shape::Line),
            "river_edge_lines" => one_d(O::RiverEdgeLines, Shape::Line),
            "river_flow_paths" => one_d(O::RiverFlowPaths, Shape::Line),
            "structures" => one_d(O::StructureCenterlines, Shape::Line),
            "storage_areas" => one_d(O::StorageAreaPolygons, Shape::Polygon),
            "river_junctions" => points(take(layer, self.river_junctions(budget)?)?),
            "pump_stations" => points(take(layer, self.pump_stations(budget)?)?),
            "pipe_conduits" => one_d(O::PipeConduits, Shape::Line),
            "pipe_nodes" => points(take(layer, self.pipe_nodes(budget)?)?),
            "reference_points" => {
                let d = take(layer, self.reference_point_geometry(budget)?)?;
                let pts = xy(layer, &d.points)?;
                let features = d
                    .features
                    .iter()
                    .map(|f| {
                        let i = usize::try_from(f.point_row)
                            .map_err(|_| refuse(UnsupportedCode::ResourceLimit, "point row"))?;
                        Ok((i..i + 1, Vec::new()))
                    })
                    .collect::<Result<Vec<_>>>()?;
                let packed = Packed {
                    features,
                    points: pts,
                    table: Some(d.attributes.table),
                };
                build(layer, Shape::Point, packed, options)
            }
            _ => Err(Error::new(
                ErrorKind::InvalidRequest,
                format!("{layer:?} is not a stored feature layer of the catalog"),
            )),
        }
    }

    /// `mesh_cell_points`: the stored `Cells Center Coordinate` of every physical cell
    /// (`mesh_name`, `cell_id`, Point `geometry`), meshes in Attributes order. Ghost cells (index
    /// at or above the area's `Cell Count`) are left out, matching `mesh_cells` and the summary
    /// layers; a mesh without a recorded `Cell Count` keeps every entry, as ras-commander's
    /// `HdfMesh.get_mesh_cell_points(include_ghost_cells=False)` does.
    pub fn mesh_cell_points_layer(&self, options: &LayerOptions) -> Result<ArrowLayer> {
        let mut names = StringBuilder::new();
        let mut ids = Vec::new();
        let mut geoms = BinaryBuilder::new();
        let mut wkb = Vec::with_capacity(21);
        let mut ghost: Vec<Option<bool>> = Vec::new();
        for mesh in self.mesh_area_names(options.metadata_max_bytes)? {
            let path = format!("{}/{mesh}/Cells Center Coordinate", super::GEOMETRY);
            let centres = match self.read_numeric_bounded(&path, None, options.max_bytes) {
                Ok(a) => a,
                Err(e) if e.kind == ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
            };
            let pts = xy("mesh_cell_points", &centres)?;
            let physical = self.physical_cell_count(&mesh, options.metadata_max_bytes)?;
            let keep = if options.include_ghost_cells {
                pts.len()
            } else {
                physical.map_or(pts.len(), |c| (c as usize).min(pts.len()))
            };
            for (i, p) in pts.iter().take(keep).enumerate() {
                if options.include_ghost_cells {
                    ghost.push(physical.map(|c| i as u64 >= c));
                }
                wkb.clear();
                wkb.push(1);
                wkb.extend_from_slice(&1u32.to_le_bytes());
                wkb.extend_from_slice(&p[0].to_le_bytes());
                wkb.extend_from_slice(&p[1].to_le_bytes());
                geoms.append_value(&wkb);
                names.append_value(&mesh);
                ids.push(
                    i64::try_from(i).map_err(|_| refuse(UnsupportedCode::ResourceLimit, "id"))?,
                );
            }
        }
        let mut fields = vec![
            Field::new("mesh_name", DataType::Utf8, false),
            Field::new("cell_id", DataType::Int64, false),
        ];
        let mut columns: Vec<ArrayRef> =
            vec![Arc::new(names.finish()), Arc::new(Int64Array::from(ids))];
        if options.include_ghost_cells {
            fields.push(Field::new("is_ghost_cell", DataType::Boolean, true));
            columns.push(Arc::new(arrow_array::BooleanArray::from(ghost)));
        }
        fields.push(geometry_field());
        columns.push(Arc::new(geoms.finish()));
        ArrowLayer::from_columns(
            "mesh_cell_points",
            fields,
            columns,
            ("mesh_name", Some("cell_id")),
            options.batch_rows,
        )
    }
}
