use super::*;
use crate::freestiler_core::mvt;
use prost::Message;

fn square(x0: f64, y0: f64, x1: f64, y1: f64, clockwise: bool) -> Vec<[f64; 2]> {
    let mut ring = vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1], [x0, y0]];
    if clockwise {
        ring.reverse();
    }
    ring
}

/// Feature 7: a polygon with a hole and an island inside the hole (two
/// parts). Feature 9: a plain polygon.
fn fixture() -> (Vec<[f64; 2]>, Vec<usize>, Vec<usize>, Vec<usize>) {
    let rings = [
        square(-40., -40., 40., 40., false),
        square(-20., -20., 20., 20., true),
        square(-10., -10., 10., 10., false),
        square(50., 10., 60., 20., false),
    ];
    let mut coords = Vec::new();
    let mut ring_offsets = vec![0];
    for ring in rings {
        coords.extend(ring);
        ring_offsets.push(coords.len());
    }
    // rings 0+1 form part one, ring 2 part two, ring 3 the second feature
    (coords, ring_offsets, vec![0, 2, 3, 4], vec![0, 2, 3])
}

fn packed<'a>(
    coords: &'a ndarray::Array2<f64>,
    ids: &'a ndarray::Array1<i64>,
    rings: Vec<usize>,
    polygons: Vec<usize>,
    features: Vec<usize>,
) -> Packed<'a> {
    Packed {
        name: "cells".into(),
        coords: coords.view(),
        ring_offsets: rings,
        polygon_offsets: polygons,
        feature_offsets: features,
        ids: ids.view(),
    }
}

fn array(coords: &[[f64; 2]]) -> ndarray::Array2<f64> {
    ndarray::Array2::from_shape_vec(
        (coords.len(), 2),
        coords.iter().flat_map(|c| c.iter().copied()).collect(),
    )
    .unwrap()
}

/// Decode MVT polygon commands into rings with their signed tile-space area.
fn rings(geometry: &[u32]) -> Vec<f64> {
    let (mut i, mut x, mut y) = (0, 0i64, 0i64);
    let mut out = Vec::new();
    let mut ring: Vec<(i64, i64)> = Vec::new();
    let zig = |v: u32| ((v >> 1) as i64) ^ -((v & 1) as i64);
    while i < geometry.len() {
        let (id, count) = (geometry[i] & 7, geometry[i] >> 3);
        i += 1;
        match id {
            1 | 2 => {
                for _ in 0..count {
                    x += zig(geometry[i]);
                    y += zig(geometry[i + 1]);
                    i += 2;
                    if id == 1 {
                        ring.clear();
                    }
                    ring.push((x, y));
                }
            }
            7 => {
                let n = ring.len();
                let area: i64 = (0..n)
                    .map(|k| {
                        let (a, b) = (ring[k], ring[(k + 1) % n]);
                        a.0 * b.1 - b.0 * a.1
                    })
                    .sum();
                out.push(area as f64 / 2.);
            }
            _ => panic!("unexpected command {id}"),
        }
    }
    out
}

#[test]
fn holes_and_parts_survive_tiling() {
    let (coords, rings_at, polygons, features) = fixture();
    let coords = array(&coords);
    let ids = ndarray::Array1::from(vec![7i64, 9]);
    let layer = build_layer(
        &packed(&coords, &ids, rings_at, polygons, features),
        vec!["label".into()],
        vec![vec![
            PropertyValue::String("holed".into()),
            PropertyValue::Null,
        ]],
        0,
        1,
    )
    .unwrap()
    .unwrap();
    match &layer.features[0].geometry {
        Geometry::MultiPolygon(mp) => {
            assert_eq!(mp.0.len(), 2);
            assert_eq!(mp.0[0].interiors().len(), 1);
            assert!(mp.0[1].interiors().is_empty());
        }
        other => panic!("expected a multipolygon, got {other:?}"),
    }
    assert!(matches!(layer.features[1].geometry, Geometry::Polygon(_)));
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("holes.pmtiles");
    crate::python::tile_engine::write(&[layer], output.to_str().unwrap(), 0, 1).unwrap();
    let mut archive =
        pmtiles2::PMTiles::from_reader(std::fs::File::open(&output).unwrap()).unwrap();
    let compressed = archive.get_tile(0, 0, 0).unwrap().expect("z0 tile");
    let mut raw = Vec::new();
    std::io::Read::read_to_end(
        &mut flate2::read::GzDecoder::new(compressed.as_slice()),
        &mut raw,
    )
    .unwrap();
    let tile = mvt::Tile::decode(raw.as_slice()).unwrap();
    let layer = &tile.layers[0];
    let holed = layer.features.iter().find(|f| f.id == Some(7)).unwrap();
    let areas = rings(&holed.geometry);
    // Exterior, hole (opposite winding), island exterior: in MVT tile space
    // (y down) exteriors have positive area and holes negative.
    assert_eq!(areas.len(), 3, "{areas:?}");
    assert!(areas[0] > 0. && areas[1] < 0. && areas[2] > 0., "{areas:?}");
    assert!(areas[0] > -areas[1] && -areas[1] > areas[2]);
    let plain = layer.features.iter().find(|f| f.id == Some(9)).unwrap();
    assert_eq!(rings(&plain.geometry).len(), 1);
}

#[test]
fn gather_takes_source_rows_and_nulls_missing_ones() {
    let values = vec![
        PropertyValue::Double(1.5),
        PropertyValue::Int(2),
        PropertyValue::String("c".into()),
    ];
    let rows = [2, -1, 0, 0];
    assert_eq!(
        gather(values.clone(), Some(&rows), "v").unwrap(),
        vec![
            PropertyValue::String("c".into()),
            PropertyValue::Null,
            PropertyValue::Double(1.5),
            PropertyValue::Double(1.5),
        ]
    );
    assert_eq!(gather(values.clone(), None, "v").unwrap(), values);
    for bad in [[3i64], [-2]] {
        let error = gather(values.clone(), Some(&bad), "v").unwrap_err();
        assert!(matches!(error, TileInputError::Value(m) if m.contains("outside the column")));
    }
}

#[test]
fn property_lengths_types_and_rings_are_checked() {
    let (coords, rings_at, polygons, features) = fixture();
    let coords = array(&coords);
    let ids = ndarray::Array1::from(vec![7i64, 9]);
    let p = || {
        packed(
            &coords,
            &ids,
            rings_at.clone(),
            polygons.clone(),
            features.clone(),
        )
    };
    let short = build_layer(
        &p(),
        vec!["v".into()],
        vec![vec![PropertyValue::Int(1)]],
        0,
        1,
    );
    assert!(matches!(short, Err(TileInputError::Value(m)) if m.contains("property length")));
    let layer = build_layer(
        &p(),
        vec!["n".into(), "s".into(), "b".into(), "m".into()],
        vec![
            vec![PropertyValue::Double(1.), PropertyValue::Int(2)],
            vec![PropertyValue::Null, PropertyValue::String("x".into())],
            vec![PropertyValue::Bool(true), PropertyValue::Null],
            vec![PropertyValue::Int(1), PropertyValue::String("x".into())],
        ],
        0,
        1,
    )
    .unwrap()
    .unwrap();
    assert_eq!(layer.prop_types, ["numeric", "string", "boolean", "mixed"]);
    // A self-intersecting ring is a topology error, not a value error.
    let bow = array(&[[0., 0.], [1., 1.], [1., 0.], [0., 1.], [0., 0.]]);
    let one = ndarray::Array1::from(vec![1i64]);
    let error = build_layer(
        &packed(&bow, &one, vec![0, 5], vec![0, 1], vec![0, 1]),
        vec![],
        vec![],
        0,
        1,
    )
    .err()
    .expect("a self-intersecting ring must refuse");
    assert!(matches!(error, TileInputError::Topology(_)), "{error:?}");
}
