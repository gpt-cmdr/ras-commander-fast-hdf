//! Supplemental single-session public-API attribution, without engine edits.
use ras_commander_fast_hdf::hdf::{
    lines::GEOMETRY_FAMILIES, structures::StructureFamilyKind, ReadOnlyHdfSession,
};
use serde_json::json;
use std::{io::Write, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let file = args.get(1).ok_or("expected file and area arguments")?;
    let area = args.get(2).ok_or("expected area argument")?;
    let t = Instant::now();
    let s = ReadOnlyHdfSession::open(file)?;
    println!(
        "{}",
        json!({"api":"open","ms":t.elapsed().as_secs_f64()*1000.})
    );
    const META: usize = 128 * 1024 * 1024;
    macro_rules! probe {
        ($name:expr,$call:expr) => {{
            let t = Instant::now();
            let result = $call;
            let elapsed = t.elapsed().as_secs_f64() * 1000.;
            let error = result.as_ref().err().map(ToString::to_string);
            let _ = std::hint::black_box(result);
            println!("{}", json!({"api":$name,"ms":elapsed,"error":error}));
            std::io::stdout().flush()?;
        }};
    }
    probe!("mesh_inventory", s.mesh_inventory(META));
    probe!("mesh_geometry", s.mesh_geometry(area, META));
    probe!("time_axis", s.unsteady_time_axis(META));
    probe!("summary_inventory", s.mesh_summary_inventory(area, META));
    probe!("series_inventory", s.mesh_series_inventory(area, META));
    probe!("boundary_inventory", s.mesh_boundary_inventory(area, META));
    for family in GEOMETRY_FAMILIES {
        probe!(
            format!("geometry_family/{}", family.as_str()),
            s.geometry_family(*family, META)
        );
    }
    probe!("cross_sections", s.cross_sections(META));
    for family in StructureFamilyKind::ALL {
        probe!(
            format!("structure_inventory/{}", family.as_str()),
            s.structure_inventory(family, META)
        );
    }
    probe!("plan_capabilities", s.plan_capabilities(META));
    probe!("run_summary", s.run_summary(META));
    Ok(())
}
