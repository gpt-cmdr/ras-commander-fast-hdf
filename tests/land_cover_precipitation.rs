#![cfg(feature = "hdf")]
//! The Final n Values rule and gridded precipitation on synthetic files
//! (`tests/fixtures/land_cover`, generator `tests/generate_land_cover_fixtures.py`).
//! Conformance with RAS Mapper and ras2cng on real models is recorded in
//! `docs/reference/land-cover-precipitation.md`.
use ras_commander_fast_hdf::hdf::ReadOnlyHdfSession;
use ras_commander_fast_hdf::land_cover::{
    ClassWindow, FinalManningsModel, Grid, TileCorner, NO_DATA,
};
use ras_commander_fast_hdf::precipitation::{PrecipitationGrid, PrecipitationSteps, Source};
use ras_commander_fast_hdf::ErrorKind;
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/land_cover")
        .join(name)
}

/// Class 1 (Grass) on the west half, 7 (Forest) on the east half, 0 (no data) in row 7.
fn render(geometry: &str, corner: TileCorner) -> (Grid, Vec<f32>) {
    let m = FinalManningsModel::open(fixture(geometry), None).unwrap();
    let source = Grid {
        min_x: 0.0,
        max_y: 8.0,
        cell_size: 1.0,
        rows: 8,
        cols: 8,
    };
    let mut classes = vec![0i32; 64];
    for r in 0..7 {
        for c in 0..8 {
            classes[r * 8 + c] = if c < 4 { 1 } else { 7 };
        }
    }
    let grid = m.export_grid(1.0).unwrap();
    assert_eq!((grid.rows, grid.cols), (8, 8));
    assert_eq!(FinalManningsModel::tile_rows(&grid), 1);
    let (r0, c0, h, w) = FinalManningsModel::class_window(&grid, 0, &source).unwrap();
    assert_eq!((r0, c0, h, w), (0, 0, 8, 8));
    let window = ClassWindow {
        data: &classes,
        row_off: 0,
        col_off: 0,
        height: 8,
        width: 8,
        source,
    };
    let out = m.render_tile_row(&grid, 0, Some(&window), corner).unwrap();
    (grid, out)
}

#[test]
fn final_n_applies_area_default_class_base_and_region_override() {
    let (_, v) = render("geom.hdf", TileCorner::RasMapper);
    // Rows 0..4, west: inside region West. Grass takes the 0.2 override.
    assert_eq!(v[0], 0.2);
    assert_eq!(v[3 * 8 + 3], 0.2);
    // Rows 0..4, east: outside the region. Forest keeps its land-cover n (table value invalid).
    assert_eq!(v[7], 0.1);
    // Rows 4..7, west: Grass takes the calibration table's base n instead of 0.03.
    assert_eq!(v[5 * 8], 0.04);
    // Row 7: class 0 keeps the 2D flow area default.
    assert_eq!(v[7 * 8], 0.5);
    assert_eq!(v[7 * 8 + 7], 0.5);
}

#[test]
fn version_1_land_cover_with_bitfield_ids_matches_the_raster_map_layout() {
    let (_, a) = render("geom.hdf", TileCorner::RasMapper);
    let (_, b) = render("geom_v1.hdf", TileCorner::RasMapper);
    assert_eq!(a, b);
    let m = FinalManningsModel::open(fixture("geom_v1.hdf"), None).unwrap();
    assert_eq!(
        m.classes.iter().map(|c| c.id).collect::<Vec<_>>(),
        vec![0, 1, 7]
    );
}

#[test]
fn no_class_window_keeps_the_area_default() {
    let m = FinalManningsModel::open(fixture("geom.hdf"), None).unwrap();
    let grid = m.export_grid(1.0).unwrap();
    let v = m
        .render_tile_row(&grid, 0, None, TileCorner::Evaluate)
        .unwrap();
    assert!(v.iter().all(|x| *x == 0.5));
    assert_ne!(v[0], NO_DATA);
}

#[test]
fn unproven_layouts_are_refused() {
    for name in ["geom_xs.hdf", "geom_multipart.hdf", "geom_virtual.hdf"] {
        let e = FinalManningsModel::open(fixture(name), None).unwrap_err();
        assert!(matches!(e.kind, ErrorKind::Unsupported), "{name}: {e:?}");
    }
}

fn steps(name: &str, indices: Option<&[usize]>, units: &str) -> PrecipitationSteps {
    let s = ReadOnlyHdfSession::open(fixture(name)).unwrap();
    let g = PrecipitationGrid::open(&s, Source::Auto).unwrap();
    PrecipitationSteps::new(s, g, indices, units, 1 << 20).unwrap()
}

#[test]
fn processed_grids_sum_missing_as_zero_and_keep_never_valid_cells_missing() {
    let mut s = steps("precip_per.hdf", None, "native");
    let g = s.grid().clone();
    assert_eq!((g.rows, g.cols, g.source.as_str()), (2, 3, "processed"));
    assert_eq!(g.cellsize, 0.1); // the float32 attribute through its shortest decimal
    assert_eq!(g.timestamps[1], "02JAN2000 00:00:00");
    let a = s.next_step().unwrap().unwrap();
    assert_eq!(a.incremental, vec![1., 2., -9999., 4., 5., -9999.]);
    assert_eq!(a.cumulative, vec![1., 2., -9999., 4., 5., 0.]);
    let b = s.next_step().unwrap().unwrap();
    assert_eq!(b.incremental, vec![1., -9999., -9999., 4., 5., 6.]);
    assert_eq!(b.cumulative, vec![2., 2., -9999., 8., 10., 6.]);
    let c = s.next_step().unwrap().unwrap();
    assert_eq!(c.cumulative, vec![3., 4., -9999., 12., 15., 12.]);
    assert!(s.next_step().unwrap().is_none());
}

#[test]
fn a_selection_returns_the_same_values_in_time_order() {
    let mut all = steps("precip_per.hdf", None, "native");
    let first = all.next_step().unwrap().unwrap();
    let _ = all.next_step().unwrap();
    let third = all.next_step().unwrap().unwrap();
    let mut s = steps("precip_per.hdf", Some(&[2, 0, 2]), "native");
    assert_eq!(s.indices(), &[2, 0]);
    assert_eq!(s.next_step().unwrap().unwrap(), first);
    assert_eq!(s.next_step().unwrap().unwrap(), third);
    assert!(s.next_step().unwrap().is_none());
}

#[test]
fn cumulative_sources_are_differenced_and_units_convert() {
    let mut s = steps("precip_cumulative.hdf", None, "in");
    assert_eq!(s.units(), Some("in"));
    assert_eq!(s.grid().timestamps, vec!["0", "1"]);
    let a = s.next_step().unwrap().unwrap();
    let inch = (1.0f64 / 25.4) as f32;
    assert_eq!(a.incremental, vec![inch; 6]);
    assert_eq!(a.cumulative, vec![inch; 6]);
    let b = s.next_step().unwrap().unwrap();
    assert_eq!(b.incremental[0], (f64::from(2.0f32) / 25.4) as f32);
    assert_eq!(b.incremental[2], 0.0);
    assert_eq!(b.cumulative[5], (f64::from(6.0f32) / 25.4) as f32);
}

#[test]
fn bad_requests_are_refused() {
    let open = || ReadOnlyHdfSession::open(fixture("precip_full.hdf")).unwrap();
    let g = PrecipitationGrid::open(&open(), Source::Auto).unwrap();
    assert!(PrecipitationGrid::open(&open(), Source::Imported).is_err());
    assert!(PrecipitationSteps::new(open(), g.clone(), Some(&[9]), "native", 1 << 20).is_err());
    assert!(PrecipitationSteps::new(open(), g.clone(), None, "ft", 1 << 20).is_err());
    // A timestep is 24 bytes.
    assert!(PrecipitationSteps::new(open(), g, None, "native", 8).is_err());
    let geometry = ReadOnlyHdfSession::open(fixture("geom.hdf")).unwrap();
    assert!(PrecipitationGrid::open(&geometry, Source::Auto).is_err());
}
