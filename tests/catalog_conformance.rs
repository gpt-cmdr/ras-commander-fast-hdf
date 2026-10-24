#![cfg(feature = "catalog")]
use ras_commander_fast_hdf::catalog::*;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

static SERIAL: AtomicUsize = AtomicUsize::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ras-catalog-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn catalog_retains_source_and_lists_errors_without_empty_success() {
    let f = Fixture::new();
    let project = f.write("model.prj", b"Proj Title=Model\r\nCurrent Plan=p01\r\nEnglish Units\r\nPlan File=p01\r\nPlan File=p02\r\nPlan File=p03\r\nGeom File=g01\r\nFlow File=f01\r\nUnsteady File=u01\r\nFuture Key=preserved\r\n");
    let raw = b"Plan Title=Unsteady\r\nShort Identifier= ID  \r\nProgram Version=7.00\r\nGeom File=g01\r\nFlow File=u01\r\nSimulation Date=01JAN3000,0000,02JAN3000,2400\r\nComputation Interval=0.5SEC\r\nOutput Interval=1HOUR\r\nBEGIN DESCRIPTION:\r\nFlow File=f99\r\nEND DESCRIPTION:\r\nUnknown=1=2\r\n";
    f.write("model.p01", raw);
    f.write("model.p03", b"garbage\n");
    f.write("model.g01", b"Geom Title=geometry\n");
    f.write("model.p01.hdf", b"presence does not validate HDF");
    let c = ProjectCatalog::open(&project).unwrap();
    assert_eq!(c.title, "Model");
    assert_eq!(c.units, ProjectUnits::English);
    assert_eq!(c.plans.len(), 3);
    assert_eq!(c.current_plan.as_ref().unwrap().path, f.0.join("model.p01"));
    let p = c.plans[0].plan.as_ref().unwrap();
    assert_eq!(p.source.bytes, raw);
    assert_eq!(p.short_identifier.as_deref(), Some("ID"));
    assert_eq!(p.source.values("Unknown").next(), Some("1=2"));
    assert_eq!(p.flow.as_ref().unwrap().kind, FlowKind::Unsteady);
    assert_eq!(p.flow.as_ref().unwrap().link.declared, "u01");
    assert_eq!(
        p.flow.as_ref().unwrap().link.availability,
        Availability::Missing
    );
    assert_eq!(
        p.geometry.as_ref().unwrap().availability,
        Availability::Present
    );
    assert_eq!(
        p.geometry_hdf.as_ref().unwrap().availability,
        Availability::Missing
    );
    assert_eq!(p.results_hdf.availability, Availability::Present);
    let simulation = p.simulation.as_ref().unwrap();
    assert_eq!(simulation.start.year, 3000);
    assert_eq!(simulation.end.hour, 24);
    assert_eq!(
        p.intervals[0].1.value,
        IntervalValue::Duration {
            amount: 0.5,
            unit: IntervalUnit::Second
        }
    );
    assert_eq!(c.plans[1].link.availability, Availability::Missing);
    assert_eq!(
        c.plans[1].plan.as_ref().unwrap_err().kind,
        CatalogErrorKind::NotFound
    );
    assert_eq!(
        c.plans[2].plan.as_ref().unwrap_err().kind,
        CatalogErrorKind::Malformed
    );
    assert_eq!(fs::read(project).unwrap(), c.source.bytes);
}

#[test]
fn relative_paths_case_and_all_flow_kinds() {
    let f = Fixture::new();
    let project = f.write("Mixed.prj", b"proj title = Paths\nSI Units=True\nCurrent Plan=Plans\\ONE.P01\nPlan File=plans/one.p01\n");
    let path = f.write(
        "Plans/ONE.P01",
        b"Plan Title=One\nGeom File=..\\Geometry\\Real.G01\nFlow File=..\\Flow\\Event.Q01\n",
    );
    f.write("Geometry/Real.G01", b"geometry");
    f.write("Flow/Event.Q01", b"flow");
    let c = ProjectCatalog::open(project).unwrap();
    assert_eq!(c.units, ProjectUnits::Si);
    assert!(c.diagnostics.is_empty());
    assert_eq!(c.plans[0].link.path, path);
    let p = c.plans[0].plan.as_ref().unwrap();
    assert_eq!(
        p.geometry.as_ref().unwrap().availability,
        Availability::Present
    );
    assert_eq!(p.flow.as_ref().unwrap().kind, FlowKind::QuasiUnsteady);
    let steady = f.write("Mixed.p02", b"Plan Title=Steady\nFlow File=f01\n");
    assert_eq!(
        Plan::open(steady).unwrap().flow.unwrap().kind,
        FlowKind::Steady
    );
}

#[test]
fn legacy_source_and_description_are_lossless() {
    let bytes =
        b"Plan Title=Caf\xe9\r\nBEGIN DESCRIPTION\r\nPlan Title=Wrong\r\nEND DESCRIPTION\r\n";
    let doc = SourceDocument::from_bytes("legacy.p01", bytes.to_vec()).unwrap();
    let p = Plan::from_document(doc).unwrap();
    assert_eq!(p.title, "Caf\u{e9}");
    assert_eq!(p.source.encoding, TextEncoding::Latin1);
    assert_eq!(p.source.bytes, bytes);
    let bom =
        SourceDocument::from_bytes("bom.p01", b"\xef\xbb\xbfPlan Title=BOM\n".to_vec()).unwrap();
    assert_eq!(Plan::from_document(bom).unwrap().title, "BOM");
}

#[test]
fn malformed_input_never_panics_or_becomes_an_empty_catalog() {
    let f = Fixture::new();
    for (index, raw) in [
        "",
        "Plan Title=A\nPlan Title=B\n",
        "Plan Title=A\nSimulation Date=29FEB3000,0000,01MAR3000,0000\n",
        "Plan Title=A\nSimulation Date=01JAN3000,2401,02JAN3000,0000\n",
        "Plan Title=A\nSimulation Date=01JAN3000,0000\n",
        "Plan Title=A\nComputation Interval=NaNSEC\n",
        "Plan Title=A\nComputation Interval=0MIN\n",
        "Plan Title=A\nBEGIN DESCRIPTION\n",
        "Plan Title=A\nEND DESCRIPTION\n",
    ]
    .iter()
    .enumerate()
    {
        let path = f.write(&format!("bad.p{index:02}"), raw.as_bytes());
        assert_eq!(
            Plan::open(path).unwrap_err().kind,
            CatalogErrorKind::Malformed,
            "{raw:?}"
        );
    }
    let path = f.write("linked.p01", b"Plan Title=A\nGeom File=oops\n");
    let p = Plan::open(path).unwrap();
    let link = p.geometry.unwrap();
    assert_eq!(link.availability, Availability::InvalidReference);
    assert_eq!(link.diagnostic.unwrap().kind, CatalogErrorKind::Malformed);
    assert_eq!(
        SourceDocument::from_bytes("bad", vec![0xff, 0xfe, 0, 0])
            .unwrap_err()
            .kind,
        CatalogErrorKind::UnsupportedEncoding
    );
    assert_eq!(
        ProjectCatalog::open(f.0.join("missing.prj"))
            .unwrap_err()
            .kind,
        CatalogErrorKind::NotFound
    );
    let conflicting = f.write("bad.prj", b"Proj Title=A\nEnglish Units\nSI Units\n");
    assert_eq!(
        ProjectCatalog::open(conflicting).unwrap_err().kind,
        CatalogErrorKind::Malformed
    );
}

#[test]
fn leap_dates_unknown_units_and_unlisted_current_plan_are_explicit() {
    let f = Fixture::new();
    let path = f.write("model.prj", b"Proj Title=A\nCurrent Plan=p99\n");
    let c = ProjectCatalog::open(path).unwrap();
    assert_eq!(c.units, ProjectUnits::Unknown);
    assert_eq!(
        c.diagnostics,
        vec![ProjectDiagnostic::CurrentPlanNotListed {
            declared: "p99".into()
        }]
    );
    let path = f.write(
        "model.p01",
        b"Plan Title=Leap\nSimulation Date=29FEB2400,0000,01MAR2400,0000\n",
    );
    assert_eq!(Plan::open(path).unwrap().simulation.unwrap().start.day, 29);
}

/// Optional immutable RasExamples qualification, deliberately ignored by default.
/// Set RAS_CATALOG_FIXTURE_ROOT to the accepted working/text-corpus directory.
/// Expectations were frozen with the accepted Python parser/DataFrame before Rust testing.
#[test]
#[ignore = "requires immutable external RasExamples text corpus; run explicitly with RAS_CATALOG_FIXTURE_ROOT"]
fn real_catalog_matches_frozen_python_and_raw_oracles() {
    use sha2::{Digest, Sha256};
    let root = PathBuf::from(
        std::env::var_os("RAS_CATALOG_FIXTURE_ROOT").expect("set RAS_CATALOG_FIXTURE_ROOT"),
    );
    let cases: serde_json::Value = serde_json::from_str(FROZEN_PYTHON_ORACLE).unwrap();
    for expected in cases.as_array().unwrap() {
        let project_path = root.join(expected["project"].as_str().unwrap());
        let plan_path = root.join(expected["plan"].as_str().unwrap());
        let before_project = fs::read(&project_path).unwrap();
        let before_plan = fs::read(&plan_path).unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(&before_project)),
            expected["project_sha256"]
        );
        assert_eq!(
            format!("{:x}", Sha256::digest(&before_plan)),
            expected["plan_sha256"]
        );
        let project = ProjectCatalog::open(&project_path).unwrap();
        assert_eq!(project.title, expected["project_title"]);
        assert_eq!(
            project.current_plan.as_ref().map(|p| p.declared.as_str()),
            expected["current_plan"].as_str()
        );
        let units = match project.units {
            ProjectUnits::English => Some("ft"),
            ProjectUnits::Si => Some("m"),
            ProjectUnits::Unknown => None,
            _ => panic!("new project units require qualification"),
        };
        assert_eq!(units, expected["units"].as_str());
        let listed: Vec<_> = project
            .plans
            .iter()
            .map(|p| p.link.declared.as_str())
            .collect();
        let expected_listed: Vec<_> = expected["listed_plans"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p.as_str().unwrap())
            .collect();
        assert_eq!(listed, expected_listed);
        let plan =
            Plan::open(&plan_path).unwrap_or_else(|e| panic!("{}: {e}", plan_path.display()));
        let fields = &expected["fields"];
        assert_eq!(plan.title, fields["Plan Title"]);
        assert_eq!(
            plan.short_identifier.as_deref(),
            fields["Short Identifier"].as_str()
        );
        assert_eq!(
            plan.program_version.as_deref(),
            fields["Program Version"].as_str()
        );
        assert_eq!(
            plan.geometry.as_ref().map(|g| g.declared.as_str()),
            fields["Geom File"].as_str()
        );
        assert_eq!(
            plan.flow.as_ref().map(|g| g.link.declared.as_str()),
            fields["Flow File"].as_str()
        );
        assert_eq!(
            plan.simulation.as_ref().map(|s| s.raw.as_str()),
            fields["Simulation Date"].as_str()
        );
        for key in [
            "Computation Interval",
            "Output Interval",
            "Mapping Interval",
            "Instantaneous Interval",
        ] {
            assert_eq!(
                plan.intervals
                    .iter()
                    .find(|(k, _)| k == key)
                    .map(|(_, v)| v.raw.as_str()),
                fields[key].as_str()
            );
        }
        // Check declared controls independently against raw bytes as well as the Python oracle.
        let text = String::from_utf8_lossy(&before_plan);
        assert!(text
            .lines()
            .any(|line| line.trim_end_matches('\r') == format!("Plan Title={}", plan.title)));
        assert_eq!(fs::read(&project_path).unwrap(), before_project);
        assert_eq!(fs::read(&plan_path).unwrap(), before_plan);
        println!(
            "qualified {} declared version {}",
            expected["plan"], fields["Program Version"]
        );
    }
}

const FROZEN_PYTHON_ORACLE: &str = r###"[
  {
    "plan": "6.6/AustinBayouSH35Firehose/AustinBayou_SH35_Firehose/RAS Model/Input/AustinOyster.p01",
    "project": "6.6/AustinBayouSH35Firehose/AustinBayou_SH35_Firehose/RAS Model/Input/AustinOyster.prj",
    "plan_sha256": "8d9b6b1cd6aab63414d81c9c5b1b85eab8f7fffd905809cdd5d20c0089783631",
    "project_sha256": "ec98378491c6d5bde5ee328287faed7725a4f5fac46865b005ded01ee556deac",
    "project_title": "AustinBayou_SH35_Firehose",
    "current_plan": "p01",
    "units": "ft",
    "listed_plans": [
      "p01"
    ],
    "fields": {
      "Plan Title": "SH35_FIREHOSE",
      "Short Identifier": "SH35FH",
      "Program Version": "5.07",
      "Geom File": "g01",
      "Flow File": "u01",
      "Simulation Date": "01JAN2000,00:00,01JAN2000,13:00",
      "Computation Interval": "30SEC",
      "Output Interval": "1HOUR",
      "Mapping Interval": "1HOUR",
      "Instantaneous Interval": "1HOUR"
    },
    "python_frame_selected": true
  },
  {
    "plan": "6.6/Balde Eagle Creek/BaldEagle.p01",
    "project": "6.6/Balde Eagle Creek/BaldEagle.prj",
    "plan_sha256": "2dd200dc4b4d42a73d15408d9337a1778aaea42332ff61b8bb641027dfe004ad",
    "project_sha256": "9e7d327fa2d0c041143c55ef02e79fc4d446bcdc665910c0f5a652c8a4dad9d5",
    "project_title": "Bald Eagle Cr. for the WCDS Project",
    "current_plan": "p01",
    "units": "ft",
    "listed_plans": [
      "p01",
      "p02"
    ],
    "fields": {
      "Plan Title": "Unsteady with Bridges and Dam",
      "Short Identifier": "UnsteadyFlow",
      "Program Version": "5.00",
      "Geom File": "g01",
      "Flow File": "u02",
      "Simulation Date": "18FEB1999,0000,24FEB1999,0500",
      "Computation Interval": "2MIN",
      "Output Interval": "1HOUR",
      "Mapping Interval": "1HOUR",
      "Instantaneous Interval": "2HOUR"
    },
    "python_frame_selected": true
  },
  {
    "plan": "6.6/BaldEagleCrkMulti2D/BaldEagleDamBrk.p01",
    "project": "6.6/BaldEagleCrkMulti2D/BaldEagleDamBrk.prj",
    "plan_sha256": "18ce65bf5f77ffcc1742a761b5ee0b5d58cf9b834c0a9d5a6d3b85955136372d",
    "project_sha256": "a112974c1216382971d60926aaf5f1d0324a4b3da0fe0309bd58e8d13f61a082",
    "project_title": "Bald Eagle Creek Example Dam Break Study",
    "current_plan": "p05",
    "units": "ft",
    "listed_plans": [
      "p13",
      "p15",
      "p17",
      "p18",
      "p19",
      "p03",
      "p04",
      "p02",
      "p01",
      "p05",
      "p06"
    ],
    "fields": {
      "Plan Title": "SA to Detailed 2D Breach FEQ",
      "Short Identifier": "SA-2D Det FEQ",
      "Program Version": "5.03",
      "Geom File": "g01",
      "Flow File": "u01",
      "Simulation Date": "01JAN1999,1200,04JAN1999,1200",
      "Computation Interval": "5SEC",
      "Output Interval": "1MIN",
      "Mapping Interval": "5MIN",
      "Instantaneous Interval": "1HOUR"
    },
    "python_frame_selected": true
  },
  {
    "plan": "6.6/BaldEagleCrkMulti2D/BaldEagleDamBrk.p02",
    "project": "6.6/BaldEagleCrkMulti2D/BaldEagleDamBrk.prj",
    "plan_sha256": "4fe57c41303a87bb1762aa87dcf491fad3b6c6c5d35e56e92d73cfc07e20b557",
    "project_sha256": "a112974c1216382971d60926aaf5f1d0324a4b3da0fe0309bd58e8d13f61a082",
    "project_title": "Bald Eagle Creek Example Dam Break Study",
    "current_plan": "p05",
    "units": "ft",
    "listed_plans": [
      "p13",
      "p15",
      "p17",
      "p18",
      "p19",
      "p03",
      "p04",
      "p02",
      "p01",
      "p05",
      "p06"
    ],
    "fields": {
      "Plan Title": "SA to Detailed 2D Breach",
      "Short Identifier": "SA-2D Det Brch",
      "Program Version": "5.10",
      "Geom File": "g01",
      "Flow File": "u01",
      "Simulation Date": "01JAN1999,1200,04JAN1999,1200",
      "Computation Interval": "10SEC",
      "Output Interval": "1MIN",
      "Mapping Interval": "5MIN",
      "Instantaneous Interval": "1HOUR"
    },
    "python_frame_selected": true
  },
  {
    "plan": "6.6/BaldEagleCrkMulti2D/BaldEagleDamBrk.p03",
    "project": "6.6/BaldEagleCrkMulti2D/BaldEagleDamBrk.prj",
    "plan_sha256": "8712e10ae157ac0e3858380aa4232e245d3138c1544df695dbccb37b20106306",
    "project_sha256": "a112974c1216382971d60926aaf5f1d0324a4b3da0fe0309bd58e8d13f61a082",
    "project_title": "Bald Eagle Creek Example Dam Break Study",
    "current_plan": "p05",
    "units": "ft",
    "listed_plans": [
      "p13",
      "p15",
      "p17",
      "p18",
      "p19",
      "p03",
      "p04",
      "p02",
      "p01",
      "p05",
      "p06"
    ],
    "fields": {
      "Plan Title": "Single 2D Area - Internal Dam Structure",
      "Short Identifier": "Single 2D",
      "Program Version": "5.04",
      "Geom File": "g09",
      "Flow File": "u13",
      "Simulation Date": "01JAN1999,1200,04JAN1999,1200",
      "Computation Interval": "30SEC",
      "Output Interval": "1MIN",
      "Mapping Interval": "10MIN",
      "Instantaneous Interval": "1HOUR"
    },
    "python_frame_selected": true
  },
  {
    "plan": "6.6/BaldEagleCrkMulti2D/BaldEagleDamBrk.p06",
    "project": "6.6/BaldEagleCrkMulti2D/BaldEagleDamBrk.prj",
    "plan_sha256": "b3bd08e6655ad8f57d4fe850a49fff4878dc12e1eb55dff1b91bc61c83934597",
    "project_sha256": "a112974c1216382971d60926aaf5f1d0324a4b3da0fe0309bd58e8d13f61a082",
    "project_title": "Bald Eagle Creek Example Dam Break Study",
    "current_plan": "p05",
    "units": "ft",
    "listed_plans": [
      "p13",
      "p15",
      "p17",
      "p18",
      "p19",
      "p03",
      "p04",
      "p02",
      "p01",
      "p05",
      "p06"
    ],
    "fields": {
      "Plan Title": "Gridded Precip - Infiltration",
      "Short Identifier": "Grid Precip Infiltration",
      "Program Version": "6.00",
      "Geom File": "g09",
      "Flow File": "u03",
      "Simulation Date": "09SEP2018,0000,14SEP2018,0000",
      "Computation Interval": "20SEC",
      "Output Interval": "1MIN",
      "Mapping Interval": "10MIN",
      "Instantaneous Interval": "1HOUR"
    },
    "python_frame_selected": true
  },
  {
    "plan": "6.6/BeaverLake/BeaverLakeSWMMImpor.p01",
    "project": "6.6/BeaverLake/BeaverLakeSWMMImpor.prj",
    "plan_sha256": "828a1a0877beedb56982a9760be3eb508d4ea6bb833caf03da199a2e2fdc393a",
    "project_sha256": "04568d3434c78c658f177470fc5a6dbb655088f2f62ac3f0f0d88ca461d66026",
    "project_title": "BeaverLakeSWMMImport",
    "current_plan": "p01",
    "units": "ft",
    "listed_plans": [
      "p03",
      "p01"
    ],
    "fields": {
      "Plan Title": "25YR_2D_WithDrainage",
      "Short Identifier": "25YR_2D_WithDrainage",
      "Program Version": "6.70",
      "Geom File": "g01",
      "Flow File": "u01",
      "Simulation Date": "11NOV2024,0900,11NOV2024,2000",
      "Computation Interval": "3SEC",
      "Output Interval": "15SEC",
      "Mapping Interval": "15SEC",
      "Instantaneous Interval": "15SEC"
    },
    "python_frame_selected": true
  },
  {
    "plan": "6.6/Chippewa_2D/Chippewa_2D.p02",
    "project": "6.6/Chippewa_2D/Chippewa_2D.prj",
    "plan_sha256": "ac81795b92392bb33a5ace38ace0dbd5c43d112b64a1718251a2674332454b9d",
    "project_sha256": "0bd2c98ce3897347a4842c6c4efa300278aab4b761d242ac3c245bdbf59e3473",
    "project_title": "Chippewa_2D",
    "current_plan": "p02",
    "units": "ft",
    "listed_plans": [
      "p02"
    ],
    "fields": {
      "Plan Title": "100ft Sediment",
      "Short Identifier": "100ft Sediment",
      "Program Version": "6.40",
      "Geom File": "g01",
      "Flow File": "u04",
      "Simulation Date": "02apr2019,0000,05may2019,2400",
      "Computation Interval": "2MIN",
      "Output Interval": "10MIN",
      "Mapping Interval": "30MIN",
      "Instantaneous Interval": "10MIN"
    },
    "python_frame_selected": true
  },
  {
    "plan": "6.6/Davis/DavisStormSystem.p02",
    "project": "6.6/Davis/DavisStormSystem.prj",
    "plan_sha256": "4c40e241858e50be823fe15981fab4a89ab54ff9899c34b89c3f780b97e208cb",
    "project_sha256": "225c1bd2da2518167a28a50a4bc3b662f012e80a93be41d160f09a148241cfe8",
    "project_title": "DavisStormSystem",
    "current_plan": "p02",
    "units": "ft",
    "listed_plans": [
      "p02"
    ],
    "fields": {
      "Plan Title": "Full System ROM with Pump",
      "Short Identifier": "Full System ROM with Pump",
      "Program Version": "6.60",
      "Geom File": "g02",
      "Flow File": "u01",
      "Simulation Date": "10JAN2000,1200,11JAN2000,2400",
      "Computation Interval": "12SEC",
      "Output Interval": "10MIN",
      "Mapping Interval": "10MIN",
      "Instantaneous Interval": "Max Profile"
    },
    "python_frame_selected": true
  },
  {
    "plan": "6.6/Balde Eagle Creek/BaldEagle.p01",
    "project": "6.6/Balde Eagle Creek/BaldEagle.prj",
    "plan_sha256": "2dd200dc4b4d42a73d15408d9337a1778aaea42332ff61b8bb641027dfe004ad",
    "project_sha256": "9e7d327fa2d0c041143c55ef02e79fc4d446bcdc665910c0f5a652c8a4dad9d5",
    "project_title": "Bald Eagle Cr. for the WCDS Project",
    "current_plan": "p01",
    "units": "ft",
    "listed_plans": [
      "p01",
      "p02"
    ],
    "fields": {
      "Plan Title": "Unsteady with Bridges and Dam",
      "Short Identifier": "UnsteadyFlow",
      "Program Version": "5.00",
      "Geom File": "g01",
      "Flow File": "u02",
      "Simulation Date": "18FEB1999,0000,24FEB1999,0500",
      "Computation Interval": "2MIN",
      "Output Interval": "1HOUR",
      "Mapping Interval": "1HOUR",
      "Instantaneous Interval": "2HOUR"
    },
    "python_frame_selected": true
  },
  {
    "plan": "7.0/Davis/DavisStormSystem.p02",
    "project": "7.0/Davis/DavisStormSystem.prj",
    "plan_sha256": "4c40e241858e50be823fe15981fab4a89ab54ff9899c34b89c3f780b97e208cb",
    "project_sha256": "225c1bd2da2518167a28a50a4bc3b662f012e80a93be41d160f09a148241cfe8",
    "project_title": "DavisStormSystem",
    "current_plan": "p02",
    "units": "ft",
    "listed_plans": [
      "p02"
    ],
    "fields": {
      "Plan Title": "Full System ROM with Pump",
      "Short Identifier": "Full System ROM with Pump",
      "Program Version": "6.60",
      "Geom File": "g02",
      "Flow File": "u01",
      "Simulation Date": "10JAN2000,1200,11JAN2000,2400",
      "Computation Interval": "12SEC",
      "Output Interval": "10MIN",
      "Mapping Interval": "10MIN",
      "Instantaneous Interval": "Max Profile"
    },
    "python_frame_selected": true
  }
]"###;

#[test]
fn colon_times_and_profile_interval_are_not_duration_guesses() {
    let f = Fixture::new();
    let path = f.write("model.p01", b"Plan Title=Profile\nSimulation Date=01JAN3000,00:00,02JAN3000,24:00\nMapping Interval=Max Profile\n");
    let plan = Plan::open(path).unwrap();
    assert_eq!(plan.simulation.as_ref().unwrap().start.raw_time, "00:00");
    assert_eq!(plan.simulation.as_ref().unwrap().end.hour, 24);
    assert_eq!(plan.intervals[0].1.value, IntervalValue::MaximumProfile);
}
