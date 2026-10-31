//! RAS Mapper's "Final n Values" raster (Manning's n after land cover, calibration
//! regions and 2D flow area defaults), computed without HEC-RAS.
//!
//! [`FinalManningsModel::open`] reads every input from the geometry HDF and its associated
//! land-cover HDF: the land-cover classification and its `ManningsN` values, the land-cover
//! classification polygons, the geometry's calibration table and calibration-region polygons,
//! and the 2D flow area perimeters with their default Manning's n. The land-cover
//! classification raster (`<land cover>.tif`) is read by the caller (the engine has no raster
//! I/O) and passed to [`FinalManningsModel::render_tile_row`] one row of 256 x 256 tiles at a
//! time, as a window of class IDs.
//!
//! The rule is RAS Mapper 7.0.1's `FinalNValueLayer.Resample` over its default *Export
//! Raster* grid (original extent = the geometry extent, cell size = the land-cover TIFF cell
//! size), tile by tile as `InterpolatedLayer.SingleFileExportRaster` evaluates it:
//!
//! 1. every pixel starts at no data (-9999);
//! 2. pixels inside a complete 2D flow area perimeter take that area's Manning's n;
//! 3. the land-cover class of each pixel is the nearest land-cover TIFF pixel (class 0 is no
//!    data), then the land-cover classification polygons are burned over it (last polygon
//!    wins);
//! 4. the calibration region of each pixel is burned (first region wins);
//! 5. the class's base n (the land-cover `ManningsN`, replaced by the calibration table's
//!    base value when that is a valid n) or the region's override replaces the value when it
//!    is a valid n.
//!
//! RAS Mapper skips step 5 for the last pixel of each 256 x 256 tile (its parallel loop runs
//! to `CellMaxIndex() - 1`); [`TileCorner::RasMapper`] reproduces that, and
//! [`TileCorner::Evaluate`] applies the rule to every pixel.
//!
//! Polygons are burned with RAS Mapper's scanline rule (pixel centres, with its vertex-on-row
//! handling), so the result is pixel-identical to RAS Mapper 7.0.1 on the qualified models.
//! Geometries with cross sections, river centerlines or pipe networks (RAS Mapper also maps
//! 1D cross-section n values), virtual (merged) land-cover layers, multi-part polygons,
//! open rings, class IDs above 255 and the HEC-RAS 5.0.3 region layout are refused.
use crate::hdf::map_error;
use crate::hdf::metadata::{CompoundTable, MetadataValue};
use crate::hdf::{NumericValues, ReadOnlyHdfSession};
use crate::{Error, ErrorKind, Result, UnsupportedCode};
use std::path::{Path, PathBuf};

/// RAS Mapper's no-data value for the exported float raster.
pub const NO_DATA: f32 = -9999.0;
/// The export tile size of `SingleFileExportRaster`.
pub const TILE: usize = 256;
const COMPOUND_MAX_BYTES: usize = 256 * 1024 * 1024;
const LC_GROUP: &str = "Geometry/Land Cover (Manning's n)";

/// Whether the last pixel of each 256 x 256 export tile takes the land-cover lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileCorner {
    /// As RAS Mapper 7.0.1: that pixel keeps the 2D-area default (or no data).
    RasMapper,
    /// Apply the lookup to every pixel.
    Evaluate,
}

/// A north-up grid: upper-left corner, square cell size, rows and columns.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grid {
    pub min_x: f64,
    pub max_y: f64,
    pub cell_size: f64,
    pub rows: usize,
    pub cols: usize,
}

/// A window of the land-cover class raster: `data` is `height x width` class IDs, row-major,
/// starting at (`row_off`, `col_off`) of the raster described by `source`.
#[derive(Debug, Clone, Copy)]
pub struct ClassWindow<'a> {
    pub data: &'a [i32],
    pub row_off: usize,
    pub col_off: usize,
    pub height: usize,
    pub width: usize,
    pub source: Grid,
}

/// One land-cover class: ID, name and the base Manning's n used for it.
#[derive(Debug, Clone, PartialEq)]
pub struct LandCoverClass {
    pub id: i32,
    pub name: String,
    /// The land-cover layer's `ManningsN` (-9999 when it has none).
    pub land_cover_n: f32,
    /// The value used: `land_cover_n`, or the calibration table's base value when valid.
    pub base_n: f32,
}

/// A 2D flow area as the rule uses it.
#[derive(Debug, Clone, PartialEq)]
pub struct FlowArea {
    pub name: String,
    pub mannings_n: f32,
    pub cell_count: i64,
}

/// Every input of the "Final n Values" rule, read from the geometry and land-cover HDF files.
#[derive(Debug, Clone)]
pub struct FinalManningsModel {
    pub geometry_hdf: PathBuf,
    pub land_cover_hdf: PathBuf,
    /// RAS Mapper's classification raster: the land-cover HDF path with extension `.tif`.
    pub land_cover_tif: PathBuf,
    pub classes: Vec<LandCoverClass>,
    pub region_names: Vec<String>,
    pub flow_areas: Vec<FlowArea>,
    /// Geometry extent (min x, max x, min y, max y): 2D flow area and storage area polygons.
    pub extent: [f64; 4],
    /// `region_n[class_row * regions + region]`, `f32::MAX` where the region does not override.
    region_n: Vec<f32>,
    id_to_row: Vec<i32>,
    /// Closed rings, RAS Mapper orientation (area > 0), in feature order.
    regions: Vec<Vec<[f64; 2]>>,
    /// Open rings after RAS Mapper's Douglas-Peucker filter, highest priority first, with class ID.
    class_polygons: Vec<(Vec<[f64; 2]>, i32)>,
    /// Closed counter-clockwise rings of complete areas, with the area n.
    perimeters: Vec<(Vec<[f64; 2]>, f32)>,
}

fn unsupported(msg: impl Into<String>) -> Error {
    Error::unsupported(UnsupportedCode::NotImplemented, msg)
}
fn schema(msg: impl Into<String>) -> Error {
    Error::unsupported(UnsupportedCode::Schema, msg)
}

fn exists(s: &ReadOnlyHdfSession, path: &str) -> Result<(bool, bool)> {
    let (parent, name) = match path.rsplit_once('/') {
        Some((p, n)) => (if p.is_empty() { "/" } else { p }, n),
        None => ("/", path),
    };
    let members = match s.group_members(parent) {
        Ok(m) => m,
        Err(e) if matches!(e.kind, ErrorKind::NotFound) => return Ok((false, false)),
        Err(e) => return Err(e),
    };
    Ok((
        members.groups.iter().any(|g| g == name),
        members.datasets.iter().any(|d| d == name),
    ))
}
pub(crate) fn has_group(s: &ReadOnlyHdfSession, p: &str) -> Result<bool> {
    // Walk parents so an absent ancestor is "absent", not an error.
    let mut acc = String::new();
    for part in p.split('/').filter(|x| !x.is_empty()) {
        let next = if acc.is_empty() {
            part.to_string()
        } else {
            format!("{acc}/{part}")
        };
        if !exists(s, &next)?.0 {
            return Ok(false);
        }
        acc = next;
    }
    Ok(true)
}
pub(crate) fn has_dataset(s: &ReadOnlyHdfSession, p: &str) -> Result<bool> {
    match p.rsplit_once('/') {
        Some((parent, _)) if !parent.is_empty() && !has_group(s, parent)? => Ok(false),
        _ => Ok(exists(s, p)?.1),
    }
}

fn rows_of(t: &CompoundTable) -> usize {
    t.shape.first().copied().unwrap_or(0) as usize
}
fn column<'a>(t: &'a CompoundTable, name: &str) -> Option<&'a MetadataValue> {
    t.columns
        .iter()
        .find(|c| c.name.eq_ignore_ascii_case(name))
        .map(|c| &c.value)
}
fn texts(t: &CompoundTable, name: &str) -> Result<Vec<String>> {
    match column(t, name) {
        Some(MetadataValue::Strings(v)) => Ok(v.clone()),
        Some(_) => Err(schema(format!("column {name:?} is not text"))),
        None => Err(schema(format!("column {name:?} is absent"))),
    }
}
fn floats32(t: &CompoundTable, name: &str) -> Result<Vec<f32>> {
    let c = t
        .columns
        .iter()
        .find(|c| c.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| schema(format!("column {name:?} is absent")))?;
    match (&c.value, &c.attribute_type) {
        (MetadataValue::Float(v), crate::hdf::types::AttributeType::Float { bytes: 4, .. }) => {
            Ok(v.iter().map(|x| *x as f32).collect())
        }
        _ => Err(schema(format!("column {name:?} is not float32"))),
    }
}
fn ints(t: &CompoundTable, name: &str) -> Result<Vec<i64>> {
    match column(t, name) {
        Some(MetadataValue::Signed(v)) => Ok(v.clone()),
        Some(MetadataValue::Unsigned(v)) => v
            .iter()
            .map(|x| i64::try_from(*x).map_err(|_| schema("integer out of range")))
            .collect(),
        _ => Err(schema(format!("column {name:?} is not integer"))),
    }
}

fn numeric_f64(s: &ReadOnlyHdfSession, path: &str) -> Result<(Vec<u64>, Vec<f64>)> {
    let a = s.read_numeric(path, None)?;
    let v = match a.values {
        NumericValues::F64(v) => v,
        NumericValues::F32(v) => v.into_iter().map(f64::from).collect(),
        _ => return Err(schema(format!("{path} is not floating point"))),
    };
    Ok((a.shape, v))
}
fn numeric_i64(s: &ReadOnlyHdfSession, path: &str) -> Result<(Vec<u64>, Vec<i64>)> {
    let a = s.read_numeric(path, None)?;
    let v: Vec<i64> = match a.values {
        NumericValues::I8(v) => v.into_iter().map(i64::from).collect(),
        NumericValues::I16(v) => v.into_iter().map(i64::from).collect(),
        NumericValues::I32(v) => v.into_iter().map(i64::from).collect(),
        NumericValues::I64(v) => v,
        NumericValues::U8(v) => v.into_iter().map(i64::from).collect(),
        NumericValues::U16(v) => v.into_iter().map(i64::from).collect(),
        NumericValues::U32(v) => v.into_iter().map(i64::from).collect(),
        _ => return Err(schema(format!("{path} is not integer"))),
    };
    Ok((a.shape, v))
}

/// Version-1 class IDs: integers, or the one-byte bitfield HEC-RAS also writes here (h5py
/// reads it as uint8).
fn class_ids(s: &ReadOnlyHdfSession, path: &str) -> Result<Vec<i64>> {
    if s.numeric_capability(path)?.kind.is_some() {
        return Ok(numeric_i64(s, path)?.1);
    }
    let ds = s.file.dataset(path).map_err(map_error)?;
    if !matches!(ds.dtype(), hdf5_reader::Datatype::Bitfield { size: 1, .. }) {
        return Err(schema(format!("{path} is not integer")));
    }
    Ok(ds
        .read_raw_bytes()
        .map_err(map_error)?
        .into_iter()
        .map(i64::from)
        .collect())
}

/// Single-part closed polygons of a `<base> Info` / `<base> Points` feature set.
fn polygons(s: &ReadOnlyHdfSession, base: &str) -> Result<Vec<Vec<[f64; 2]>>> {
    let (ishape, info) = numeric_i64(s, &format!("{base} Info"))?;
    let (pshape, pts) = numeric_f64(s, &format!("{base} Points"))?;
    if ishape.len() != 2 || ishape[1] != 4 || pshape.len() != 2 || pshape[1] != 2 {
        return Err(schema(format!("{base}: unexpected Info/Points shape")));
    }
    let npts = pshape[0] as usize;
    let mut out = Vec::new();
    for r in info.as_chunks::<4>().0 {
        let (start, count, part_count) = (r[0], r[1], r[3]);
        if part_count != 1 {
            return Err(unsupported(format!("{base}: multi-part polygon")));
        }
        if start < 0 || count < 4 || (start + count) as usize > npts {
            return Err(Error::new(
                ErrorKind::Corrupt,
                format!("{base}: polygon point range out of bounds"),
            ));
        }
        let ring: Vec<[f64; 2]> = (start as usize..(start + count) as usize)
            .map(|i| [pts[2 * i], pts[2 * i + 1]])
            .collect();
        if ring.first() != ring.last() || ring.iter().flatten().any(|v| !v.is_finite()) {
            return Err(unsupported(format!(
                "{base}: open or non-finite polygon ring"
            )));
        }
        out.push(ring);
    }
    Ok(out)
}

/// RAS Mapper `Polygon.Area` (negative of the shoelace sum).
fn ras_area(ring: &[[f64; 2]]) -> f64 {
    let min_y = ring.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min);
    let mut a = 0.0;
    for w in ring.windows(2) {
        a += 0.5 * ((w[0][1] - min_y) + (w[1][1] - min_y)) * (w[1][0] - w[0][0]);
    }
    a
}

/// `PolygonFeatureLayer.EnsurePositivePolygonOrientations`.
fn ensure_positive(mut ring: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
    if ras_area(&ring) < 0.0 {
        ring.reverse();
    }
    ring
}

/// `PointMs.FilterXY` (Douglas-Peucker) on a single-part closed ring.
fn filter_xy(ring: &[[f64; 2]], tolerance: f64) -> Vec<[f64; 2]> {
    let n = ring.len();
    if tolerance.is_nan() || n <= 2 {
        return ring.to_vec();
    }
    let (first, last) = (0usize, n - 1);
    let mut keep = vec![first, last];
    let mut last_unique = last;
    for j in (first..=last).rev() {
        if ring[first] != ring[j] {
            last_unique = j;
            break;
        }
    }
    let tol = tolerance.max(1e-12);
    fn dp(p: &[[f64; 2]], f: usize, lu: usize, tl: usize, tol: f64, keep: &mut Vec<usize>) {
        if tl - f <= 1 {
            return;
        }
        let dx = p[lu][0] - p[f][0];
        let dy = p[lu][1] - p[f][1];
        let (mut best, mut bi) = (0.0f64, 0usize);
        for (i, q) in p.iter().enumerate().take(tl + 1).skip(f + 1) {
            let d = (dx * (q[1] - p[f][1]) - dy * (q[0] - p[f][0])).abs();
            if d > best {
                best = d;
                bi = i;
            }
        }
        let len = (dx.powi(2) + dy.powi(2)).sqrt();
        if best / len > tol {
            keep.push(bi);
            dp(p, f, bi, bi, tol, keep);
            dp(p, bi, lu, tl, tol, keep);
        }
    }
    dp(ring, first, last_unique, last, tol, &mut keep);
    keep.sort_unstable();
    keep.dedup();
    keep.into_iter().map(|i| ring[i]).collect()
}

fn valid_n(v: f32) -> bool {
    v > 0.0 && v != f32::MAX && !v.is_nan()
}
fn ascii_upper(s: &str) -> Result<String> {
    if !s.is_ascii() {
        return Err(unsupported(format!(
            "non-ASCII land-cover name {s:?} (case-insensitive matching unproven)"
        )));
    }
    Ok(s.to_ascii_uppercase())
}

fn string_attr(s: &ReadOnlyHdfSession, group: &str, name: &str) -> Result<Option<String>> {
    let attrs = s.typed_group_attributes(group)?;
    match attrs.iter().find(|a| a.name == name) {
        None => Ok(None),
        Some(a) => match &a.value {
            MetadataValue::Strings(v) if v.len() == 1 => Ok(Some(v[0].clone())),
            _ => Err(schema(format!("attribute {name:?} is not one string"))),
        },
    }
}

impl FinalManningsModel {
    /// Read the rule's inputs. `land_cover_hdf` overrides the geometry's association.
    pub fn open(geometry_hdf: impl AsRef<Path>, land_cover_hdf: Option<&Path>) -> Result<Self> {
        let geometry_hdf = geometry_hdf.as_ref().to_path_buf();
        let g = ReadOnlyHdfSession::open(&geometry_hdf)?;
        if !has_group(&g, "Geometry")? {
            return Err(schema("no Geometry group"));
        }
        for (path, what) in [
            ("Geometry/Cross Sections/Attributes", "cross sections"),
            ("Geometry/River Centerlines/Attributes", "river centerlines"),
            ("Geometry/Pipe Conduits/Attributes", "pipe conduits"),
            ("Geometry/Pipe Nodes/Attributes", "pipe nodes"),
            ("Geometry/Pipe Networks/Attributes", "pipe networks"),
        ] {
            if has_dataset(&g, path)? && rows_of(&g.simple_compound(path, COMPOUND_MAX_BYTES)?) > 0
            {
                return Err(unsupported(format!(
                    "geometry has {what}: RAS Mapper's 1D cross-section n mapping and extent are not implemented"
                )));
            }
        }
        let land_cover_hdf = match land_cover_hdf {
            Some(p) => p.to_path_buf(),
            None => {
                let rel = string_attr(&g, "Geometry", "Land Cover Filename")?
                    .filter(|v| !v.trim().is_empty())
                    .ok_or_else(|| {
                        Error::new(
                            ErrorKind::NotFound,
                            "geometry has no Land Cover Filename association",
                        )
                    })?;
                let rel = rel.trim().replace('\\', "/");
                let p = Path::new(&rel);
                if p.is_absolute() {
                    p.to_path_buf()
                } else {
                    geometry_hdf
                        .parent()
                        .unwrap_or_else(|| Path::new("."))
                        .join(p)
                }
            }
        };
        if !land_cover_hdf
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("hdf"))
        {
            return Err(unsupported(
                "land-cover association is not an .hdf layer (RAS Mapper converts it; unproven)",
            ));
        }
        let land_cover_tif = land_cover_hdf.with_extension("tif");
        let l = ReadOnlyHdfSession::open(&land_cover_hdf)?;
        let root = l.typed_group_attributes("/")?;
        if root.iter().any(|a| a.name == "Virtual Dependencies") {
            return Err(unsupported(
                "virtual (merged) land-cover layer is not implemented",
            ));
        }
        // Classification (insertion order, as .NET's Dictionary) and the ManningsN parameter.
        let mut classes: Vec<(i32, String)> = Vec::new();
        let mut mannings: Vec<(String, f32)> = Vec::new();
        let insert = |classes: &mut Vec<(i32, String)>, id: i64, name: String| -> Result<()> {
            let id = i32::try_from(id).map_err(|_| schema("class ID out of range"))?;
            if let Some(e) = classes.iter_mut().find(|c| c.0 == id) {
                e.1 = name;
            } else {
                classes.push((id, name));
            }
            Ok(())
        };
        if has_dataset(&l, "Raster Map")? {
            let rm = l.simple_compound("Raster Map", COMPOUND_MAX_BYTES)?;
            for (id, name) in ints(&rm, "ID")?.into_iter().zip(texts(&rm, "Name")?) {
                insert(&mut classes, id, name)?;
            }
            if has_dataset(&l, "Variables")? {
                let v = l.simple_compound("Variables", COMPOUND_MAX_BYTES)?;
                let names = texts(&v, "Name")?;
                if let Some(c) = v
                    .columns
                    .iter()
                    .find(|c| c.name.eq_ignore_ascii_case("ManningsN"))
                {
                    let vals = floats32(&v, &c.name)?;
                    for (n, x) in names.into_iter().zip(vals) {
                        if let Some(e) = mannings.iter_mut().find(|m| m.0 == n) {
                            e.1 = x;
                        } else {
                            mannings.push((n, x));
                        }
                    }
                }
            }
        } else if has_dataset(&l, "IDs")? && has_dataset(&l, "Names")? {
            if root.iter().any(|a| a.name == "Has Raster File") {
                return Err(unsupported(
                    "version-1 land cover with a 'Has Raster File' attribute",
                ));
            }
            let ids = class_ids(&l, "IDs")?;
            let names = l.read_strings("Names")?;
            if ids.len() != names.len() {
                return Err(Error::new(ErrorKind::Corrupt, "IDs/Names length mismatch"));
            }
            for (id, n) in ids.iter().zip(&names) {
                insert(&mut classes, *id, n.clone())?;
            }
            if has_dataset(&l, "ManningsN")? {
                let (_, v) = numeric_f64(&l, "ManningsN")?;
                if v.len() == ids.len() {
                    for (n, x) in names.iter().zip(v) {
                        let x = x as f32;
                        if !(x == -9999.0 || x == f32::MAX || x.is_nan()) {
                            mannings.push((n.clone(), x));
                        }
                    }
                }
            }
        } else {
            return Err(unsupported(
                "land-cover HDF without a raster classification (no Raster Map or IDs/Names)",
            ));
        }
        // Classification polygons: new classes take the first free IDs, then the polygons.
        let mut class_polygons = Vec::new();
        if has_group(&l, "Classification Polygons")? {
            let attrs =
                l.simple_compound("Classification Polygons/Attributes", COMPOUND_MAX_BYTES)?;
            let names = texts(&attrs, "Classification")?;
            let rings = polygons(&l, "Classification Polygons/Polygon")?;
            if rings.len() != names.len() {
                return Err(Error::new(
                    ErrorKind::Corrupt,
                    "classification polygon count does not match its attributes",
                ));
            }
            let valid = |n: &str| {
                !n.is_empty()
                    && !n.contains('/')
                    && !n.contains('\\')
                    && !n.eq_ignore_ascii_case("NoData")
            };
            let get_id = |classes: &[(i32, String)], n: &str| -> Result<i32> {
                let u = ascii_upper(n)?;
                if u == "NODATA" {
                    return Ok(0);
                }
                for (id, name) in classes {
                    if ascii_upper(name)? == u {
                        return Ok(*id);
                    }
                }
                Ok(-1)
            };
            // GetValidClassificationNames is a HashSet: unique names, first-seen order.
            let mut uniq: Vec<&String> = Vec::new();
            for n in &names {
                if valid(n) && !uniq.contains(&n) {
                    uniq.push(n);
                }
            }
            if uniq.len() > 1 {
                return Err(unsupported(
                    "more than one new classification-polygon class (HashSet ID order unproven)",
                ));
            }
            let mut next = 1i32;
            for n in uniq {
                if get_id(&classes, n)? != -1 {
                    continue;
                }
                while classes.iter().any(|c| c.0 == next && !c.1.is_empty()) {
                    next += 1;
                }
                classes.push((next, n.clone()));
                next += 1;
            }
            for i in (0..rings.len()).rev() {
                let n = &names[i];
                if !valid(n) {
                    continue;
                }
                let id = get_id(&classes, n)?;
                if id > 0 {
                    let ring = ensure_positive(rings[i].clone());
                    let (mut x0, mut x1, mut y0, mut y1) = (
                        f64::INFINITY,
                        f64::NEG_INFINITY,
                        f64::INFINITY,
                        f64::NEG_INFINITY,
                    );
                    for p in &ring {
                        x0 = x0.min(p[0]);
                        x1 = x1.max(p[0]);
                        y0 = y0.min(p[1]);
                        y1 = y1.max(p[1]);
                    }
                    let tol = (x1 - x0).max(y1 - y0) / 1e5;
                    let mut f = filter_xy(&ring, tol);
                    f.pop(); // Converter.Convert drops the closing point
                    if f.len() < 3 {
                        return Err(unsupported(
                            "classification polygon degenerate after filtering",
                        ));
                    }
                    class_polygons.push((f, id));
                }
            }
        }
        if classes.iter().any(|c| !(0..=255).contains(&c.0)) {
            return Err(unsupported(
                "land-cover class ID outside 0..=255 (RAS Mapper indexes a 256-entry table)",
            ));
        }
        let mut id_to_row = vec![-1i32; 256];
        for (row, (id, name)) in classes.iter().enumerate() {
            // Array.IndexOf(names, name): the first class with this exact name.
            let first = classes.iter().position(|c| &c.1 == name).unwrap_or(row);
            id_to_row[*id as usize] = first as i32;
        }
        let mut out_classes: Vec<LandCoverClass> = classes
            .iter()
            .map(|(id, name)| {
                let n = mannings
                    .iter()
                    .find(|m| &m.0 == name)
                    .map(|m| m.1)
                    .unwrap_or(NO_DATA);
                LandCoverClass {
                    id: *id,
                    name: name.clone(),
                    land_cover_n: n,
                    base_n: n,
                }
            })
            .collect();

        // Calibration regions and table.
        let mut region_names = Vec::new();
        let mut regions = Vec::new();
        if has_group(&g, LC_GROUP)? {
            if has_dataset(&g, &format!("{LC_GROUP}/Attributes"))? {
                let a = g.simple_compound(&format!("{LC_GROUP}/Attributes"), COMPOUND_MAX_BYTES)?;
                region_names = texts(&a, "Name")?;
                if rows_of(&a) > 0 {
                    regions = polygons(&g, &format!("{LC_GROUP}/Polygon"))?
                        .into_iter()
                        .map(ensure_positive)
                        .collect();
                }
                if regions.len() != region_names.len() {
                    return Err(Error::new(
                        ErrorKind::Corrupt,
                        "calibration region count does not match its attributes",
                    ));
                }
            } else if has_dataset(&g, &format!("{LC_GROUP}/Region Points"))? {
                return Err(unsupported("HEC-RAS 5.0.3 calibration region layout"));
            }
        }
        let nreg = regions.len();
        let mut region_n = vec![f32::MAX; out_classes.len() * nreg];
        let table = format!("{LC_GROUP}/Calibration Table");
        if has_dataset(&g, &table)? {
            let t = g.simple_compound(&table, COMPOUND_MAX_BYTES)?;
            let names = texts(&t, "Land Cover Name")?;
            let base = floats32(&t, "Base Manning's n Value")?;
            // BaseNames setter: OrdinalIgnoreCase dictionary, later rows win.
            let mut dict: Vec<(String, f32)> = Vec::new();
            for (n, v) in names.iter().zip(&base) {
                let u = ascii_upper(n)?;
                if let Some(e) = dict.iter_mut().find(|d| d.0 == u) {
                    e.1 = *v;
                } else {
                    dict.push((u, *v));
                }
            }
            let mut columns = Vec::new();
            for r in &region_names {
                // A region without a column keeps no mapping.
                columns.push(
                    match t.columns.iter().find(|c| c.name.eq_ignore_ascii_case(r)) {
                        Some(c) => Some(floats32(&t, &c.name)?),
                        None => None,
                    },
                );
            }
            for (row, class) in out_classes.iter_mut().enumerate() {
                let u = ascii_upper(&class.name)?;
                let mut v = f32::NAN;
                if class.name == "NoData" {
                    if let Some(e) = dict.iter().find(|d| d.0.is_empty()) {
                        v = e.1;
                    }
                }
                if let Some(e) = dict.iter().find(|d| d.0 == u) {
                    v = e.1;
                }
                if valid_n(v) {
                    class.base_n = v;
                }
                for (r, col) in columns.iter().enumerate() {
                    let Some(col) = col else { continue };
                    let mut found = None;
                    for (n, x) in names.iter().zip(col) {
                        if ascii_upper(n)? == u {
                            found = Some(*x);
                        }
                    }
                    if let Some(x) = found.filter(|x| valid_n(*x)) {
                        region_n[row * nreg + r] = x;
                    }
                }
            }
        }

        // 2D flow areas.
        let mut flow_areas = Vec::new();
        let mut perimeters = Vec::new();
        let mut ext = [
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ];
        let mut grow = |ring: &[[f64; 2]]| {
            for p in ring {
                ext[0] = ext[0].min(p[0]);
                ext[1] = ext[1].max(p[0]);
                ext[2] = ext[2].min(p[1]);
                ext[3] = ext[3].max(p[1]);
            }
        };
        let attrs_path = "Geometry/2D Flow Areas/Attributes";
        if has_dataset(&g, attrs_path)? {
            let a = g.simple_compound(attrs_path, COMPOUND_MAX_BYTES)?;
            let names = texts(&a, "Name")?;
            let mann = floats32(&a, "Mann")?;
            let cells = ints(&a, "Cell Count")?;
            let rings = polygons(&g, "Geometry/2D Flow Areas/Polygon")?;
            if rings.len() != names.len() {
                return Err(Error::new(
                    ErrorKind::Corrupt,
                    "2D flow area polygon count does not match its attributes",
                ));
            }
            for (i, ring) in rings.into_iter().enumerate() {
                grow(&ring);
                flow_areas.push(FlowArea {
                    name: names[i].clone(),
                    mannings_n: mann[i],
                    cell_count: cells[i],
                });
                if cells[i] <= 0 {
                    return Err(unsupported(format!(
                        "2D flow area {:?} has no computed mesh (completeness unproven)",
                        names[i]
                    )));
                }
                // MeshFV2D perimeters are counter-clockwise (RAS area < 0).
                let mut ring = ring;
                if ras_area(&ring) > 0.0 {
                    ring.reverse();
                }
                let n = mann[i];
                if !(n < 0.0 || n == f32::MAX || n.is_nan()) {
                    perimeters.push((ring, n));
                }
            }
        }
        if has_dataset(&g, "Geometry/Storage Areas/Polygon Points")? {
            let (shape, pts) = numeric_f64(&g, "Geometry/Storage Areas/Polygon Points")?;
            if shape.len() != 2 || shape[1] != 2 {
                return Err(schema("storage area points shape"));
            }
            let ring: Vec<[f64; 2]> = pts.as_chunks::<2>().0.to_vec();
            grow(&ring);
        }
        if !ext.iter().all(|v| v.is_finite()) || ext[1] <= ext[0] || ext[3] <= ext[2] {
            return Err(unsupported(
                "geometry has no 2D flow area or storage area extent",
            ));
        }
        Ok(Self {
            geometry_hdf,
            land_cover_hdf,
            land_cover_tif,
            classes: out_classes,
            region_names,
            flow_areas,
            extent: ext,
            region_n,
            id_to_row,
            regions,
            class_polygons,
            perimeters,
        })
    }

    /// RAS Mapper's default Export Raster grid at `cell_size` (the smallest land-cover TIFF
    /// cell size): upper-left at the geometry extent's (min x, max y).
    pub fn export_grid(&self, cell_size: f64) -> Result<Grid> {
        if !(cell_size.is_finite() && cell_size > 0.0) {
            return Err(Error::new(
                ErrorKind::InvalidRequest,
                "cell size must be > 0",
            ));
        }
        let count = |axis: f64| -> usize {
            let n = axis / cell_size;
            let r = n.round();
            let tol = n.abs().max(1.0) * 1e-9;
            let c = if (n - r).abs() <= tol { r } else { n.ceil() };
            (c as usize).max(1)
        };
        Ok(Grid {
            min_x: self.extent[0],
            max_y: self.extent[3],
            cell_size,
            rows: count(self.extent[3] - self.extent[2]),
            cols: count(self.extent[1] - self.extent[0]),
        })
    }

    /// Number of 256-row tile rows of `grid`.
    pub fn tile_rows(grid: &Grid) -> usize {
        grid.rows.div_ceil(TILE)
    }

    fn tiles(grid: &Grid, tile_row: usize) -> Vec<Tile> {
        let nt = grid.rows.div_ceil(TILE);
        let nw = grid.cols.div_ceil(TILE);
        let cs = grid.cell_size;
        let t_max_x = grid.min_x + nw as f64 * cs * TILE as f64;
        let t_min_y = grid.max_y - nt as f64 * cs * TILE as f64;
        // RasterM(rows, cols, maxX, minX, maxY, minY): cell size from its extent.
        let cs2 = ((t_max_x - grid.min_x) / nw as f64).max((grid.max_y - t_min_y) / nt as f64);
        (0..nw)
            .map(|l| {
                let min_x = grid.min_x + cs2 * l as f64;
                let max_x = grid.min_x + cs2 * (l + 1) as f64;
                let max_y = grid.max_y - cs2 * tile_row as f64;
                let min_y = grid.max_y - cs2 * (tile_row + 1) as f64;
                let tcs = ((max_x - min_x) / TILE as f64).max((max_y - min_y) / TILE as f64);
                Tile {
                    m: Ras {
                        min_x,
                        max_x,
                        min_y,
                        max_y,
                        cs: tcs,
                        rows: TILE,
                        cols: TILE,
                    },
                    d: Ras {
                        min_x,
                        max_x: min_x + TILE as f64 * tcs,
                        min_y: max_y - TILE as f64 * tcs,
                        max_y,
                        cs: tcs,
                        rows: TILE,
                        cols: TILE,
                    },
                }
            })
            .collect()
    }

    /// The class-raster window (`row_off`, `col_off`, `height`, `width`) that
    /// [`render_tile_row`](Self::render_tile_row) needs for `tile_row`, clipped to `source`;
    /// `None` when the tile row does not overlap the raster.
    pub fn class_window(
        grid: &Grid,
        tile_row: usize,
        source: &Grid,
    ) -> Option<(usize, usize, usize, usize)> {
        let (mut r0, mut r1, mut c0, mut c1) = (usize::MAX, 0usize, usize::MAX, 0usize);
        for t in Self::tiles(grid, tile_row) {
            let (rows, cols) = sample_indices(&t.d, source);
            for r in rows.iter().flatten() {
                r0 = r0.min(*r);
                r1 = r1.max(*r);
            }
            for c in cols.iter().flatten() {
                c0 = c0.min(*c);
                c1 = c1.max(*c);
            }
        }
        (r0 != usize::MAX && c0 != usize::MAX).then(|| (r0, c0, r1 - r0 + 1, c1 - c0 + 1))
    }

    /// Evaluate one row of 256 x 256 export tiles. Returns `min(256, rows - 256*tile_row)`
    /// rows of `grid.cols` values, row-major.
    pub fn render_tile_row(
        &self,
        grid: &Grid,
        tile_row: usize,
        classes: Option<&ClassWindow<'_>>,
        corner: TileCorner,
    ) -> Result<Vec<f32>> {
        if tile_row >= Self::tile_rows(grid) {
            return Err(Error::new(
                ErrorKind::InvalidRequest,
                "tile row out of range",
            ));
        }
        if let Some(w) = classes {
            if w.data.len() != w.height.saturating_mul(w.width) {
                return Err(Error::new(
                    ErrorKind::InvalidRequest,
                    "class window size does not match its shape",
                ));
            }
        }
        let first = tile_row * TILE;
        let nrows = TILE.min(grid.rows - first);
        let mut out = vec![NO_DATA; nrows * grid.cols];
        let nreg = self.regions.len();
        for (l, t) in Self::tiles(grid, tile_row).iter().enumerate() {
            let mut nv = vec![NO_DATA; TILE * TILE];
            let mut lc = vec![0i32; TILE * TILE];
            if let Some(w) = classes {
                let (rows, cols) = sample_indices(&t.d, &w.source);
                for (i, r) in rows.iter().enumerate() {
                    let Some(r) = r else { continue };
                    for (j, c) in cols.iter().enumerate() {
                        let Some(c) = c else { continue };
                        if *r < w.row_off
                            || *c < w.col_off
                            || *r - w.row_off >= w.height
                            || *c - w.col_off >= w.width
                        {
                            return Err(Error::new(
                                ErrorKind::InvalidRequest,
                                "class window does not cover the tile row",
                            ));
                        }
                        let v = w.data[(*r - w.row_off) * w.width + (*c - w.col_off)];
                        if v != 0 {
                            lc[i * TILE + j] = v;
                        }
                    }
                }
            }
            // ClassificationResampler: lowest priority first.
            for (ring, id) in self.class_polygons.iter().rev() {
                scanline(&t.d, &PolyRef::open(ring), |r, a, b| {
                    lc[r * TILE + a..=r * TILE + b].fill(*id);
                });
            }
            let mut rf = vec![-1i32; TILE * TILE];
            for i in (0..nreg).rev() {
                scanline(&t.m, &PolyRef::closed(&self.regions[i]), |r, a, b| {
                    rf[r * TILE + a..=r * TILE + b].fill(i as i32);
                });
            }
            for (ring, n) in &self.perimeters {
                if t.m.overlaps(ring) {
                    scanline(&t.m, &PolyRef::closed(ring), |r, a, b| {
                        nv[r * TILE + a..=r * TILE + b].fill(*n);
                    });
                }
            }
            let last = TILE * TILE - 1;
            for k in 0..TILE * TILE {
                if k == last && corner == TileCorner::RasMapper {
                    continue;
                }
                let m = self.lookup(lc[k], rf[k])?;
                if m != f32::MAX && m != NO_DATA && m >= 0.0 {
                    nv[k] = m;
                }
            }
            let col0 = l * TILE;
            let ncols = TILE.min(grid.cols - col0);
            for r in 0..nrows {
                out[r * grid.cols + col0..r * grid.cols + col0 + ncols]
                    .copy_from_slice(&nv[r * TILE..r * TILE + ncols]);
            }
        }
        Ok(out)
    }

    /// `FinalNValueLayer.GetManningsFromTable`.
    fn lookup(&self, id: i32, region: i32) -> Result<f32> {
        if !(0..=255).contains(&id) {
            return Err(unsupported("land-cover raster value outside 0..=255"));
        }
        let row = self.id_to_row[id as usize];
        if id == 0 && region == -1 && row == -1 {
            return Ok(NO_DATA);
        }
        if row == -1 {
            return Ok(NO_DATA);
        }
        let base = self.classes[row as usize].base_n;
        if region < 0 {
            return Ok(base);
        }
        let v = self.region_n[row as usize * self.regions.len() + region as usize];
        Ok(if v == f32::MAX || v <= 0.0 { base } else { v })
    }
}

/// RasterM / RasterDefinition arithmetic.
#[derive(Debug, Clone, Copy)]
struct Ras {
    min_x: f64,
    max_x: f64,
    min_y: f64,
    max_y: f64,
    cs: f64,
    rows: usize,
    cols: usize,
}
struct Tile {
    /// RasMapperLib `RasterM` (extent as assigned).
    m: Ras,
    /// Geospatial `RasterDefinition` (extent from upper-left, cell size and counts).
    d: Ras,
}
impl Ras {
    fn row(&self, y: f64) -> i64 {
        clamp_i32(((self.max_y - y) / self.cs).floor())
    }
    fn col(&self, x: f64) -> i64 {
        clamp_i32(((x - self.min_x) / self.cs).floor())
    }
    fn ccx(&self, c: i64) -> f64 {
        self.min_x + self.cs * (c as f64 + 0.5)
    }
    fn ccy(&self, r: i64) -> f64 {
        self.max_y - self.cs * (r as f64 + 0.5)
    }
    fn rows_in_range(&self, lo: f64, hi: f64) -> Option<(i64, i64)> {
        if lo >= self.max_y || hi <= self.min_y {
            return None;
        }
        Some((self.row(hi).max(0), self.row(lo).min(self.rows as i64 - 1)))
    }
    fn overlaps(&self, ring: &[[f64; 2]]) -> bool {
        let (mut x0, mut x1, mut y0, mut y1) = (
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
        );
        for p in ring {
            x0 = x0.min(p[0]);
            x1 = x1.max(p[0]);
            y0 = y0.min(p[1]);
            y1 = y1.max(p[1]);
        }
        !(self.max_x < x0 || self.min_x > x1 || self.min_y > y1 || self.max_y < y0)
    }
}
fn clamp_i32(v: f64) -> i64 {
    if v >= 2147483647.0 {
        i32::MAX as i64
    } else if v <= -2147483648.0 {
        i32::MIN as i64
    } else {
        v as i64
    }
}

/// Nearest source row and column for each tile pixel row/column (`Resample_Nearest`); `None`
/// where the pixel centre is outside the source raster.
fn sample_indices(t: &Ras, s: &Grid) -> (Vec<Option<usize>>, Vec<Option<usize>>) {
    let num = t.min_x + 0.5 * t.cs;
    let num2 = t.max_y - 0.5 * t.cs;
    let rows = (0..t.rows)
        .map(|i| {
            let cy = num2 - t.cs * i as f64;
            let r = ((s.max_y - cy).max(0.0) / s.cell_size).floor();
            (cy <= s.max_y && r < s.rows as f64).then_some(r as usize)
        })
        .collect();
    let cols = (0..t.cols)
        .map(|j| {
            let cx = num + t.cs * j as f64;
            let c = ((cx - s.min_x) / s.cell_size).floor();
            (c >= 0.0 && c < s.cols as f64).then_some(c as usize)
        })
        .collect();
    (rows, cols)
}

/// A ring with RasMapperLib (`closed`: last == first, `GetNextPointOnPart` wrap to 1) or
/// Geospatial (`open`: cyclic) vertex indexing.
struct PolyRef<'a> {
    p: &'a [[f64; 2]],
    closed: bool,
}
impl<'a> PolyRef<'a> {
    fn closed(p: &'a [[f64; 2]]) -> Self {
        Self { p, closed: true }
    }
    fn open(p: &'a [[f64; 2]]) -> Self {
        Self { p, closed: false }
    }
    fn segments(&self) -> usize {
        if self.closed {
            self.p.len() - 1
        } else {
            self.p.len()
        }
    }
    fn next(&self, i: usize) -> usize {
        let n = self.p.len();
        if self.closed {
            if i + 1 == n {
                1
            } else {
                i + 1
            }
        } else {
            (i + 1) % n
        }
    }
    fn prev(&self, i: usize) -> usize {
        let n = self.p.len();
        if self.closed {
            if i == 0 {
                n - 2
            } else {
                i - 1
            }
        } else {
            (n + i - 1) % n
        }
    }
    fn vertically_between(&self, i: usize) -> bool {
        let y = self.p[i][1];
        let a = self.p[self.next(i)][1] - y;
        let b = self.p[self.prev(i)][1] - y;
        if a == 0.0 && b == 0.0 {
            return false;
        }
        if a == 0.0 {
            return b > 0.0;
        }
        if b == 0.0 {
            return a > 0.0;
        }
        a.signum() != b.signum()
    }
}

/// `RasterizePolygon.ComputeCells` / `PolygonRasterizer.Rasterize`: calls `fill(row, c0, c1)`.
fn scanline(g: &Ras, poly: &PolyRef<'_>, mut fill: impl FnMut(usize, usize, usize)) {
    let p = poly.p;
    let (mut pmin, mut pmax) = (f64::INFINITY, f64::NEG_INFINITY);
    for q in p {
        pmin = pmin.min(q[1]);
        pmax = pmax.max(q[1]);
    }
    let r0 = g.row(pmax.min(g.max_y)).max(0);
    let r1 = g.row(pmin.max(g.min_y)).min(g.rows as i64 - 1);
    if r1 < r0 {
        return;
    }
    let mut rowd: Vec<Vec<f64>> = vec![Vec::new(); (r1 - r0 + 1) as usize];
    let at = |r: i64| (r - r0) as usize;
    let eps = g.cs * 1e-5;
    let add_unique = |l: &mut Vec<f64>, x: f64| {
        if !l.contains(&x) {
            l.push(x);
        }
    };
    let remove = |l: &mut Vec<f64>, x: f64| {
        if let Some(k) = l.iter().position(|v| *v == x) {
            l.remove(k);
        }
    };
    for k in 0..poly.segments() {
        let (ia, ib) = (
            k,
            if poly.closed {
                k + 1
            } else {
                (k + 1) % p.len()
            },
        );
        let (a, b) = (p[ia], p[ib]);
        let mut dy = b[1] - a[1];
        if dy == 0.0 {
            continue;
        }
        let (mut y0, x0, mut y1, x1, i0, i1);
        if dy > 0.0 {
            (y0, x0, y1, x1) = (a[1], a[0], b[1], b[0]);
            i0 = ia;
            i1 = poly.next(i0);
        } else {
            (y0, x0, y1, x1) = (b[1], b[0], a[1], a[0]);
            i1 = ia;
            i0 = poly.next(i1);
            dy = -dy;
        }
        let Some((mn, mx)) = g.rows_in_range(y0, y1) else {
            continue;
        };
        let slope = (x1 - x0) / dy;
        let cy = g.ccy(mn);
        if y1 == cy {
            y1 -= eps;
        }
        if y0 == cy {
            y0 -= eps;
        }
        {
            let l = &mut rowd[at(mn)];
            if y1 == cy {
                if x1 < g.max_x {
                    if poly.vertically_between(i1) {
                        add_unique(l, x1);
                    } else {
                        remove(l, x1);
                    }
                }
            } else if y0 == cy {
                if x0 < g.max_x {
                    if poly.vertically_between(i0) {
                        add_unique(l, x0);
                    } else {
                        remove(l, x0);
                    }
                }
            } else if y1 > cy && y0 < cy {
                let xx = x0 + (cy - y0) * slope;
                if xx < g.max_x {
                    l.push(xx);
                }
            }
        }
        if mx <= mn {
            continue;
        }
        let cy2 = g.ccy(mx);
        if y0 == cy2 {
            y0 -= eps;
        }
        {
            let l = &mut rowd[at(mx)];
            if y0 == cy2 {
                if x0 < g.max_x {
                    if poly.vertically_between(i0) {
                        add_unique(l, x0);
                    } else {
                        remove(l, x0);
                    }
                }
            } else if y0 < cy2 {
                let xx = x0 + (cy2 - y0) * slope;
                if xx < g.max_x {
                    l.push(xx);
                }
            }
        }
        for r in mn + 1..mx {
            let xx = x0 + (g.ccy(r) - y0) * slope;
            if xx < g.max_x {
                rowd[at(r)].push(xx);
            }
        }
    }
    for (k, l) in rowd.iter_mut().enumerate() {
        if l.is_empty() {
            continue;
        }
        let r = r0 + k as i64;
        l.sort_by(|a, b| a.total_cmp(b));
        if l.len() % 2 == 1 {
            l.push(g.max_x);
        }
        for pair in l.as_chunks::<2>().0 {
            let (a, b) = (pair[0], pair[1]);
            if b <= g.min_x {
                continue;
            }
            let c0 = if a < g.min_x {
                0
            } else {
                let c = g.col(a);
                if a > g.ccx(c) {
                    c + 1
                } else {
                    c
                }
            };
            let c1 = if b >= g.max_x {
                g.cols as i64 - 1
            } else {
                let c = g.col(b);
                if b < g.ccx(c) {
                    c - 1
                } else {
                    c
                }
            };
            let c0 = c0.max(0);
            let c1 = c1.min(g.cols as i64 - 1);
            if c1 >= c0 {
                fill(r as usize, c0 as usize, c1 as usize);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<[f64; 2]> {
        vec![[x0, y0], [x0, y1], [x1, y1], [x1, y0], [x0, y0]]
    }
    fn grid10() -> Ras {
        Ras {
            min_x: 0.0,
            max_x: 10.0,
            min_y: 0.0,
            max_y: 10.0,
            cs: 1.0,
            rows: 10,
            cols: 10,
        }
    }
    fn burn(g: &Ras, p: &PolyRef<'_>) -> Vec<u8> {
        let mut m = vec![0u8; g.rows * g.cols];
        scanline(g, p, |r, a, b| m[r * g.cols + a..=r * g.cols + b].fill(1));
        m
    }

    #[test]
    fn scanline_burns_pixel_centres_inside() {
        let g = grid10();
        let ring = square(2.2, 3.2, 5.8, 7.8);
        let m = burn(&g, &PolyRef::closed(&ring));
        // Centres x in {2.5..5.5} (cols 2..5), y in {3.5..7.5} (rows 2..6).
        for r in 0..10 {
            for c in 0..10 {
                let inside = (2..=5).contains(&c) && (2..=6).contains(&r);
                assert_eq!(m[r * 10 + c] == 1, inside, "r{r} c{c}");
            }
        }
        let mut open = ring.clone();
        open.pop();
        assert_eq!(m, burn(&g, &PolyRef::open(&open)));
    }

    #[test]
    fn scanline_vertex_on_centre_row_counts_once() {
        let g = grid10();
        // A diamond whose left/right vertices lie exactly on the row-4 centre line (y = 5.5).
        let ring = vec![[5.0, 9.0], [8.0, 5.5], [5.0, 2.0], [2.0, 5.5], [5.0, 9.0]];
        let m = burn(&g, &PolyRef::closed(&ring));
        let row4: Vec<u8> = m[40..50].to_vec();
        assert_eq!(row4, vec![0, 0, 1, 1, 1, 1, 1, 1, 0, 0]);
    }

    #[test]
    fn scanline_clips_to_the_raster() {
        let g = grid10();
        let m = burn(&g, &PolyRef::closed(&square(-5.0, -5.0, 15.0, 15.0)));
        assert!(m.iter().all(|v| *v == 1));
        let m = burn(&g, &PolyRef::closed(&square(20.0, 20.0, 30.0, 30.0)));
        assert!(m.iter().all(|v| *v == 0));
    }

    #[test]
    fn filter_xy_drops_collinear_points_only() {
        let ring = vec![
            [0.0, 0.0],
            [0.0, 5.0],
            [0.0, 10.0],
            [10.0, 10.0],
            [10.0, 0.0],
            [0.0, 0.0],
        ];
        let f = filter_xy(&ring, 1e-4);
        assert_eq!(
            f,
            vec![
                [0.0, 0.0],
                [0.0, 10.0],
                [10.0, 10.0],
                [10.0, 0.0],
                [0.0, 0.0]
            ]
        );
    }

    #[test]
    fn orientation_rules() {
        let ccw = vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [0.0, 0.0]];
        assert!(ras_area(&ccw) < 0.0);
        let pos = ensure_positive(ccw.clone());
        assert!(ras_area(&pos) > 0.0);
        assert_eq!(pos[0], ccw[0]);
    }

    #[test]
    fn export_grid_counts_like_rasmapper() {
        let mut m = FinalManningsModel {
            geometry_hdf: PathBuf::new(),
            land_cover_hdf: PathBuf::new(),
            land_cover_tif: PathBuf::new(),
            classes: vec![],
            region_names: vec![],
            flow_areas: vec![],
            extent: [0.0, 16.5, 0.0, 1.0],
            region_n: vec![],
            id_to_row: vec![-1; 256],
            regions: vec![],
            class_polygons: vec![],
            perimeters: vec![],
        };
        let g = m.export_grid(0.009999999776482582).unwrap();
        assert_eq!((g.rows, g.cols), (101, 1651));
        m.extent = [0.0, 10.0, 0.0, 10.0];
        let g = m.export_grid(1.0).unwrap();
        assert_eq!((g.rows, g.cols), (10, 10));
        assert!(m.export_grid(0.0).is_err());
    }

    #[test]
    fn tile_corner_quirk_and_lookup() {
        let m = FinalManningsModel {
            geometry_hdf: PathBuf::new(),
            land_cover_hdf: PathBuf::new(),
            land_cover_tif: PathBuf::new(),
            classes: vec![
                LandCoverClass {
                    id: 0,
                    name: "NoData".into(),
                    land_cover_n: NO_DATA,
                    base_n: NO_DATA,
                },
                LandCoverClass {
                    id: 7,
                    name: "Grass".into(),
                    land_cover_n: 0.05,
                    base_n: 0.05,
                },
            ],
            region_names: vec![],
            flow_areas: vec![],
            extent: [0.0, 300.0, 0.0, 300.0],
            region_n: vec![],
            id_to_row: {
                let mut v = vec![-1; 256];
                v[0] = 0;
                v[7] = 1;
                v
            },
            regions: vec![],
            class_polygons: vec![],
            perimeters: vec![(square(0.0, 0.0, 300.0, 300.0), 0.03)],
        };
        let g = m.export_grid(1.0).unwrap();
        let src = Grid {
            min_x: 0.0,
            max_y: 300.0,
            cell_size: 1.0,
            rows: 300,
            cols: 300,
        };
        let data = vec![7i32; 300 * 300];
        let (r0, c0, h, w) = FinalManningsModel::class_window(&g, 0, &src).unwrap();
        assert_eq!((r0, c0), (0, 0));
        let win: Vec<i32> = (0..h)
            .flat_map(|r| data[(r0 + r) * 300 + c0..(r0 + r) * 300 + c0 + w].to_vec())
            .collect();
        let cw = ClassWindow {
            data: &win,
            row_off: r0,
            col_off: c0,
            height: h,
            width: w,
            source: src,
        };
        let a = m
            .render_tile_row(&g, 0, Some(&cw), TileCorner::RasMapper)
            .unwrap();
        assert_eq!(a.len(), 256 * 300);
        assert_eq!(a[255 * 300 + 255], 0.03); // the skipped corner keeps the area n
        assert_eq!(a[255 * 300 + 254], 0.05);
        assert_eq!(a[255 * 300 + 299], 0.05); // second tile's corner is beyond the grid
        let b = m
            .render_tile_row(&g, 0, Some(&cw), TileCorner::Evaluate)
            .unwrap();
        assert_eq!(b[255 * 300 + 255], 0.05);
        let short = ClassWindow {
            height: 1,
            data: &win[..w],
            ..cw
        };
        assert!(m
            .render_tile_row(&g, 0, Some(&short), TileCorner::RasMapper)
            .is_err());
        assert!(m
            .render_tile_row(&g, 5, Some(&cw), TileCorner::RasMapper)
            .is_err());
    }
}
