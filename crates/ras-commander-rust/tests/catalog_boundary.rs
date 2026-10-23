#![cfg(feature = "catalog")]
use ras_commander_rust::catalog::{Availability, ProjectCatalog};
#[test]
fn untrusted_roots_are_inventory_without_access() {
    let dir = std::env::temp_dir().join(format!("catalog-boundary-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("plans")).unwrap();
    std::fs::write(
        dir.join("plans/a.p01"),
        "Plan Title=A\nGeom File=../a.g01\nFlow File=../../outside.u01\n",
    )
    .unwrap();
    std::fs::write(dir.join("a.g01"), "geometry").unwrap();
    for bad in [
        r"\x.p01",
        "/x.p01",
        "C:x.p01",
        r"\\?\C:\x.p01",
        r"\\unreachable.invalid\share\x.p01",
        "//unreachable.invalid/share/x.p01",
        "C:/elsewhere/x.p01",
        "../outside.p01",
        "invalid",
    ] {
        std::fs::write(
            dir.join("a.prj"),
            format!("Proj Title=A\nPlan File={bad}\nPlan File=plans/a.p01\n"),
        )
        .unwrap();
        let c = ProjectCatalog::open(dir.join("a.prj")).unwrap();
        assert_ne!(c.plans[0].link.availability, Availability::Present, "{bad}");
        assert!(c.plans[0].plan.is_err());
        assert!(c.plans[0].link.diagnostic.is_some());
        let good = c.plans[1].plan.as_ref().unwrap();
        assert_eq!(
            good.geometry.as_ref().unwrap().availability,
            Availability::Present
        );
        assert_eq!(
            good.flow.as_ref().unwrap().link.availability,
            Availability::OutsideProject
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[cfg(windows)]
fn differently_cased_project_path_preserves_dependencies() {
    let dir = std::env::temp_dir().join(format!("CaseBoundary-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("Plans")).unwrap();
    std::fs::write(
        dir.join("Model.prj"),
        "Proj Title=A\nPlan File=Plans/Model.p01\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("Plans/Model.p01"),
        "Plan Title=A\nGeom File=../Model.g01\nFlow File=../Model.u01\n",
    )
    .unwrap();
    for file in ["Model.g01", "Model.u01"] {
        std::fs::write(dir.join(file), "source").unwrap();
    }
    let typed = dir.join("Model.prj").to_string_lossy().to_ascii_uppercase();
    assert_ne!(typed, dir.join("Model.prj").to_string_lossy());
    let c = ProjectCatalog::open(typed).unwrap();
    assert_eq!(c.plans[0].link.availability, Availability::Present);
    let p = c.plans[0].plan.as_ref().unwrap();
    assert_eq!(
        p.geometry.as_ref().unwrap().availability,
        Availability::Present
    );
    assert_eq!(
        p.flow.as_ref().unwrap().link.availability,
        Availability::Present
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn invalid_flow_and_derived_hdf_errors_stay_local() {
    use ras_commander_rust::catalog::{FlowKind, Plan, SourceDocument};
    let dir = std::env::temp_dir().join(format!("derived-error-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("block"), "not a directory").unwrap();
    let doc = SourceDocument::from_bytes(
        dir.join("block/a.p01"),
        b"Plan Title=A\nFlow File=invalid\n".to_vec(),
    )
    .unwrap();
    let p = Plan::from_document(doc).unwrap();
    assert_eq!(p.flow.unwrap().kind, FlowKind::Unknown);
    assert_eq!(p.results_hdf.availability, Availability::InvalidReference);
    assert!(p.results_hdf.diagnostic.is_some());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[cfg(windows)]
fn unicode_cased_project_root_preserves_containment() {
    let dir = std::env::temp_dir().join(format!("CäséBoundary-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("Pläns")).unwrap();
    std::fs::write(
        dir.join("Mödel.prj"),
        "Proj Title=A\nPlan File=Pläns/Mödel.p01\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("Pläns/Mödel.p01"),
        "Plan Title=A\nGeom File=../Dätä.g01\nFlow File=../Dätä.u01\n",
    )
    .unwrap();
    for f in ["Dätä.g01", "Dätä.u01"] {
        std::fs::write(dir.join(f), "data").unwrap();
    }
    let c = ProjectCatalog::open(dir.join("Mödel.prj").to_str().unwrap().to_uppercase()).unwrap();
    let p = c.plans[0].plan.as_ref().unwrap();
    assert_eq!(
        p.geometry.as_ref().unwrap().availability,
        Availability::Present
    );
    assert_eq!(
        p.flow.as_ref().unwrap().link.availability,
        Availability::Present
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn unicode_confusables_cannot_escape_or_redirect_links() {
    let dir = std::env::temp_dir().join(format!("negative-unicode-{}", std::process::id()));
    for (i, (inside, outside)) in [
        ("Projects", "Projectſ"),
        ("fi", "fı"),
        ("σ", "ς"),
        ("µ", "μ"),
        ("ǆ", "ǅ"),
        ("ა", "Ა"),
    ]
    .into_iter()
    .enumerate()
    {
        let parent = dir.join(i.to_string());
        let root = parent.join(inside);
        std::fs::create_dir_all(root.join("Plans")).unwrap();
        std::fs::create_dir_all(parent.join(outside)).unwrap();
        std::fs::write(parent.join(outside).join("x.g01"), "outside").unwrap();
        std::fs::write(root.join("a.prj"), "Proj Title=A\nPlan File=Plans/a.p01\n").unwrap();
        std::fs::write(
            root.join("Plans/a.p01"),
            format!("Plan Title=A\nGeom File=../../{outside}/x.g01\n"),
        )
        .unwrap();
        let c = ProjectCatalog::open(root.join("a.prj")).unwrap();
        assert_eq!(
            c.plans[0]
                .plan
                .as_ref()
                .unwrap()
                .geometry
                .as_ref()
                .unwrap()
                .availability,
            Availability::OutsideProject,
            "{inside}/{outside}"
        );
    }
    let root = dir.join("redirect");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("fıle.g01"), "wrong target").unwrap();
    std::fs::write(root.join("a.prj"), "Proj Title=A\nPlan File=a.p01\n").unwrap();
    std::fs::write(root.join("a.p01"), "Plan Title=A\nGeom File=file.g01\n").unwrap();
    let c = ProjectCatalog::open(root.join("a.prj")).unwrap();
    assert_eq!(
        c.plans[0]
            .plan
            .as_ref()
            .unwrap()
            .geometry
            .as_ref()
            .unwrap()
            .availability,
        Availability::Missing
    );
    std::fs::remove_dir_all(dir).unwrap();
}
