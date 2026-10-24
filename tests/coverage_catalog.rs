//! Project and plan text edge cases for the catalog feature: encodings, line endings, missing and
//! unknown keys, unicode and escaping paths, duplicates. Every case asserts an explicit status.
#![cfg(feature = "catalog")]
use ras_commander_fast_hdf::catalog::*;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

static SERIAL: AtomicUsize = AtomicUsize::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ras-coverage-catalog-{}-{}",
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
fn doc(bytes: &[u8]) -> Result<SourceDocument, CatalogError> {
    SourceDocument::from_bytes("probe.p01", bytes.to_vec())
}
fn plan(bytes: &[u8]) -> Result<Plan, CatalogError> {
    Plan::from_document(doc(bytes)?)
}
fn kind<T: std::fmt::Debug>(r: Result<T, CatalogError>) -> CatalogErrorKind {
    r.unwrap_err().kind
}
fn pairs(d: &SourceDocument) -> Vec<(String, Option<String>)> {
    d.records
        .iter()
        .map(|r| (r.key.clone(), r.value.clone()))
        .collect()
}

#[test]
fn line_endings_do_not_change_records_and_bytes_are_retained() {
    let lf = b"Proj Title=Line Ends\nCurrent Plan=p01\nEnglish Units\nPlan File=p01\n".to_vec();
    let crlf: Vec<u8> = String::from_utf8(lf.clone())
        .unwrap()
        .replace('\n', "\r\n")
        .into_bytes();
    let mixed =
        b"Proj Title=Line Ends\r\nCurrent Plan=p01\nEnglish Units\r\nPlan File=p01".to_vec();
    let blank_lines =
        b"\n\nProj Title=Line Ends\n   \nCurrent Plan=p01\n\t\nEnglish Units\nPlan File=p01\n\n"
            .to_vec();
    let base = pairs(&SourceDocument::from_bytes("m.prj", lf.clone()).unwrap());
    for (label, bytes) in [("crlf", crlf), ("mixed", mixed), ("blank", blank_lines)] {
        let d = SourceDocument::from_bytes("m.prj", bytes.clone()).unwrap();
        assert_eq!(pairs(&d), base, "{label}");
        assert_eq!(d.bytes, bytes, "{label}: original bytes retained");
        assert_eq!(d.encoding, TextEncoding::Utf8);
    }
    let f = Fixture::new();
    for (label, bytes) in [
        ("lf", lf.clone()),
        (
            "crlf-end",
            b"Proj Title=Line Ends\r\nPlan File=p01\r\n".to_vec(),
        ),
    ] {
        let project = f.write(&format!("{label}.prj"), &bytes);
        f.write(&format!("{label}.p01"), b"Plan Title=P\r\n");
        let c = ProjectCatalog::open(project).unwrap();
        assert_eq!(c.title, "Line Ends", "{label}");
        assert_eq!(c.plans.len(), 1);
        assert_eq!(c.plans[0].plan.as_ref().unwrap().title, "P");
    }
}

#[test]
fn bom_and_unsupported_encodings_have_explicit_statuses() {
    let d = doc(b"\xef\xbb\xbfPlan Title=BOM\r\nGeom File=g01\r\n").unwrap();
    assert_eq!(d.encoding, TextEncoding::Utf8);
    assert_eq!(
        d.records[0].key, "Plan Title",
        "BOM is not part of the first key"
    );
    assert_eq!(
        d.bytes[..3],
        [0xef, 0xbb, 0xbf],
        "BOM bytes stay in the retained source"
    );
    assert_eq!(Plan::from_document(d).unwrap().title, "BOM");
    // A BOM with no content is an explicit missing-key error.
    assert_eq!(kind(plan(b"\xef\xbb\xbf")), CatalogErrorKind::Malformed);
    assert_eq!(kind(plan(b"")), CatalogErrorKind::Malformed);
    // UTF-16 (either byte order) and embedded NUL are unsupported, never decoded as Latin-1.
    for bytes in [
        b"\xff\xfeP\x00l\x00a\x00n\x00".to_vec(),
        b"\xfe\xff\x00P\x00l\x00a\x00n".to_vec(),
        b"Plan Title=A\x00B\n".to_vec(),
        b"\xef\xbb\xbfPlan Title=A\x00\n".to_vec(),
    ] {
        assert_eq!(
            kind(SourceDocument::from_bytes("u.p01", bytes.clone())),
            CatalogErrorKind::UnsupportedEncoding,
            "{bytes:?}"
        );
    }
    // Legacy Latin-1 payload: named fallback, lossless bytes, characters decoded one byte each.
    let latin = plan(b"Plan Title=Caf\xe9 \xfc\xdf\r\nShort Identifier=\xb5\n").unwrap();
    assert_eq!(latin.source.encoding, TextEncoding::Latin1);
    assert_eq!(latin.title, "Caf\u{e9} \u{fc}\u{df}");
    assert_eq!(latin.short_identifier.as_deref(), Some("\u{b5}"));
    // Valid multi-byte UTF-8 is never mistaken for Latin-1.
    let utf8 = plan("Plan Title=Caf\u{e9} \u{65e5}\u{672c}\n".as_bytes()).unwrap();
    assert_eq!(utf8.source.encoding, TextEncoding::Utf8);
    assert_eq!(utf8.title, "Caf\u{e9} \u{65e5}\u{672c}");
    // Truncated UTF-8 sequence falls back to Latin-1 rather than panicking.
    let truncated = plan(b"Plan Title=ends \xe6\x97").unwrap();
    assert_eq!(truncated.source.encoding, TextEncoding::Latin1);
}

#[test]
fn bom_followed_by_legacy_bytes_is_not_silently_misparsed() {
    // the BOM bytes as three characters, so the first key is not recognised. The result is
    // an explicit Malformed status, not a plan with a wrong or empty title.
    let e = plan(b"\xef\xbb\xbfPlan Title=Caf\xe9\n").unwrap_err();
    assert_eq!(e.kind, CatalogErrorKind::Malformed);
    assert!(e.message.contains("missing Plan Title"), "{e}");
    let d = doc(b"\xef\xbb\xbfPlan Title=Caf\xe9\n").unwrap();
    assert_eq!(d.encoding, TextEncoding::Latin1);
    assert!(d.records[0].key.ends_with("Plan Title") && d.records[0].key != "Plan Title");
}

#[test]
fn missing_optional_keys_are_none_and_missing_required_keys_are_malformed() {
    let p = plan(b"Plan Title=Only Title\n").unwrap();
    assert_eq!(p.title, "Only Title");
    assert!(p.short_identifier.is_none() && p.program_version.is_none());
    assert!(p.geometry.is_none() && p.geometry_hdf.is_none() && p.flow.is_none());
    assert!(p.simulation.is_none() && p.intervals.is_empty());
    assert_eq!(p.results_hdf.declared, "derived: append .hdf");
    assert_eq!(p.results_hdf.availability, Availability::Missing);
    assert_eq!(p.results_hdf.path, Path::new("probe.p01.hdf"));
    for bytes in [
        &b"Short Identifier=X\n"[..],
        b"Plan Title=\n",
        b"Plan Title=   \n",
        b"Plan Title\n",
    ] {
        assert_eq!(kind(plan(bytes)), CatalogErrorKind::Malformed, "{bytes:?}");
    }
    let e = plan(b"Geom File=g01\n").unwrap_err();
    assert!(e.message.contains("Plan Title"), "{e}");
    // Empty optional values are absent, not empty strings.
    let p = plan(b"Plan Title=T\nShort Identifier=\nProgram Version=   \nGeom File=\n").unwrap();
    assert!(p.short_identifier.is_none() && p.program_version.is_none() && p.geometry.is_none());
    let f = Fixture::new();
    let project = f.write("bare.prj", b"Proj Title=Bare\n");
    let c = ProjectCatalog::open(project).unwrap();
    assert_eq!(c.units, ProjectUnits::Unknown);
    assert!(c.plans.is_empty() && c.current_plan.is_none() && c.program_version.is_none());
    assert!(c.geometries.is_empty() && c.steady_flows.is_empty());
    assert!(
        c.unsteady_flows.is_empty()
            && c.quasi_unsteady_flows.is_empty()
            && c.diagnostics.is_empty()
    );
    let no_title = f.write("untitled.prj", b"Current Plan=p01\nPlan File=p01\n");
    assert_eq!(
        kind(ProjectCatalog::open(no_title)),
        CatalogErrorKind::Malformed
    );
}

#[test]
fn unknown_keys_whitespace_and_case_are_preserved_and_tolerated() {
    let d = doc(b"  plan title  =  Spaced Title  \nFuture Key=a=b=c\nBare Flag\nEmpty=\n=novalue\nFuture Key=a=b=c\n").unwrap();
    let all = pairs(&d);
    assert_eq!(all[0], ("plan title".into(), Some("Spaced Title".into())));
    assert_eq!(
        all[1],
        ("Future Key".into(), Some("a=b=c".into())),
        "value keeps later equals signs"
    );
    assert_eq!(all[2], ("Bare Flag".into(), None));
    assert_eq!(all[3], ("Empty".into(), Some(String::new())));
    assert_eq!(all[4], ("".into(), Some("novalue".into())));
    assert_eq!(
        d.values("FUTURE KEY").count(),
        2,
        "key lookup is case-insensitive"
    );
    assert_eq!(d.records[0].line, 1);
    assert_eq!(d.records[5].line, 6);
    let p = Plan::from_document(d).unwrap();
    assert_eq!(p.title, "Spaced Title");
    assert_eq!(p.source.values("future key").next(), Some("a=b=c"));
}

#[test]
fn duplicate_keys_agree_or_conflict_explicitly() {
    let ok = plan(b"Plan Title=T\nplan title=T\nGeom File=g01\nGEOM FILE=g01\n").unwrap();
    assert_eq!(ok.geometry.unwrap().declared, "g01");
    for bytes in [
        &b"Plan Title=A\nPLAN TITLE=B\n"[..],
        b"Plan Title=T\nGeom File=g01\nGeom File=g02\n",
        b"Plan Title=T\nFlow File=u01\nFlow File=f01\n",
        b"Plan Title=T\nOutput Interval=1HOUR\nOutput Interval=2HOUR\n",
    ] {
        let e = plan(bytes).unwrap_err();
        assert_eq!(e.kind, CatalogErrorKind::Malformed, "{bytes:?}");
        assert!(e.message.contains("conflicting"), "{e}");
    }
}

#[test]
fn description_blocks_hide_control_keys_and_must_balance() {
    let p = plan(b"Plan Title=Real\nbegin description:\nPlan Title=Fake\nGeom File=zzz\nend description:\nGeom File=g01\n").unwrap();
    assert_eq!(p.title, "Real");
    assert_eq!(p.geometry.unwrap().declared, "g01");
    let p = plan(
        b"Plan Title=Real\r\n  BEGIN DESCRIPTION  \r\nFlow File=bogus\r\n  END DESCRIPTION\r\n",
    )
    .unwrap();
    assert!(
        p.flow.is_none(),
        "keys inside a description are not control keys"
    );
    assert!(p.source.values("Flow File").next().is_none());
    for bytes in [
        &b"Plan Title=T\nBEGIN DESCRIPTION:\nBEGIN DESCRIPTION:\nEND DESCRIPTION:\n"[..],
        b"Plan Title=T\nEND DESCRIPTION:\n",
        b"Plan Title=T\nBEGIN DESCRIPTION:\nnever closed\n",
        b"BEGIN DESCRIPTION:\nPlan Title=Hidden\nEND DESCRIPTION:\n",
    ] {
        assert_eq!(kind(plan(bytes)), CatalogErrorKind::Malformed, "{bytes:?}");
    }
}

#[test]
fn project_unit_declarations_resolve_or_conflict() {
    let f = Fixture::new();
    let units = |body: &str| {
        let p = f.write("u.prj", format!("Proj Title=U\n{body}").as_bytes());
        ProjectCatalog::open(p).map(|c| c.units)
    };
    assert_eq!(units("English Units\n").unwrap(), ProjectUnits::English);
    assert_eq!(units("SI Units\n").unwrap(), ProjectUnits::Si);
    for v in ["True", "1", "YES", "true"] {
        assert_eq!(
            units(&format!("SI Units={v}\n")).unwrap(),
            ProjectUnits::Si,
            "{v}"
        );
    }
    for v in ["False", "0", "no"] {
        assert_eq!(
            units(&format!("SI Units={v}\n")).unwrap(),
            ProjectUnits::English,
            "{v}"
        );
    }
    assert_eq!(
        units("English Units\nSI Units=False\n").unwrap(),
        ProjectUnits::English
    );
    assert_eq!(
        units("SI Units=true\nSI Units=true\n").unwrap(),
        ProjectUnits::Si
    );
    assert_eq!(units("Other=1\n").unwrap(), ProjectUnits::Unknown);
    for body in [
        "SI Units=maybe\n",
        "SI Units=\n",
        "English Units\nSI Units=True\n",
        "SI Units=true\nSI Units=false\n",
    ] {
        assert_eq!(
            units(body).unwrap_err().kind,
            CatalogErrorKind::Malformed,
            "{body:?}"
        );
    }
    // The English marker is only a bare record; a valued record is not a unit declaration.
    assert_eq!(
        units("English Units=True\n").unwrap(),
        ProjectUnits::Unknown
    );
}

#[test]
fn unicode_paths_resolve_for_utf8_and_latin1_sources() {
    let f = Fixture::new();
    let project = f.write("Mod\u{e8}le \u{65e5}\u{672c}.prj", "Proj Title=\u{65e5}\u{672c}\nPlan File=p01\nPlan File=sub dir/\u{e9}t\u{e9}.p02\nGeom File=\u{30b8}\u{30aa}.g01\n".as_bytes());
    f.write(
        "Mod\u{e8}le \u{65e5}\u{672c}.p01",
        "Plan Title=Un\u{ef}code\nGeom File=\u{30b8}\u{30aa}.g01\n".as_bytes(),
    );
    f.write("sub dir/\u{e9}t\u{e9}.p02", b"Plan Title=Second\n");
    f.write("\u{30b8}\u{30aa}.g01", b"geometry");
    let c = ProjectCatalog::open(&project).unwrap();
    assert_eq!(c.title, "\u{65e5}\u{672c}");
    assert_eq!(c.plans.len(), 2);
    assert_eq!(c.plans[0].link.availability, Availability::Present);
    assert_eq!(c.plans[1].link.availability, Availability::Present);
    assert_eq!(c.geometries[0].availability, Availability::Present);
    let p = c.plans[0].plan.as_ref().unwrap();
    assert_eq!(p.title, "Un\u{ef}code");
    assert_eq!(
        p.geometry.as_ref().unwrap().availability,
        Availability::Present
    );
    assert_eq!(
        p.geometry_hdf.as_ref().unwrap().availability,
        Availability::Missing
    );
    // Latin-1 source naming a Latin-1 decodable file name.
    let legacy = f.write("legacy.prj", b"Proj Title=Legacy\nGeom File=G\xe9o.g01\n");
    f.write("G\u{e9}o.g01", b"geometry");
    let c = ProjectCatalog::open(legacy).unwrap();
    assert_eq!(c.source.encoding, TextEncoding::Latin1);
    assert_eq!(c.geometries[0].declared, "G\u{e9}o.g01");
    assert_eq!(c.geometries[0].availability, Availability::Present);
}

#[test]
fn invalid_file_references_have_per_link_diagnostics() {
    let f = Fixture::new();
    for (i, bad) in [
        "Plan File=notes.txt",
        "Plan File=p",
        "Plan File=x01",
        "Plan File=model.g01",
        "Plan File=",
        "Plan File=   ",
        "Plan File=model.p",
        "Plan File=model.p1x",
        "Plan File=C:plans.p01",
        "Current Plan=notes.txt",
        "Geom File=model.p01",
        "Flow File=model.g01",
        "Unsteady File=model.f01",
        "QuasiUnsteady File=model.u01",
    ]
    .iter()
    .enumerate()
    {
        let project = f.write(
            &format!("bad{i}.prj"),
            format!("Proj Title=Bad\n{bad}\n").as_bytes(),
        );
        let c = ProjectCatalog::open(project).unwrap();
        assert!(!c.diagnostics.is_empty(), "{bad}");
    }
    let plan_file = f.write("bad.p01", b"Plan Title=T\nGeom File=model.f01\n");
    assert_eq!(
        Plan::open(plan_file)
            .unwrap()
            .geometry
            .unwrap()
            .availability,
        Availability::InvalidReference
    );
    let flow_file = f.write("bad2.p01", b"Plan Title=T\nFlow File=model.g01\n");
    assert_eq!(
        Plan::open(flow_file)
            .unwrap()
            .flow
            .unwrap()
            .link
            .availability,
        Availability::InvalidReference
    );
}

#[test]
fn missing_and_non_file_targets_have_availability_statuses() {
    let f = Fixture::new();
    let project = f.write("m.prj", b"Proj Title=M\nPlan File=p01\nPlan File=p02\nPlan File=p03\nGeom File=g01\nGeom File=g02\n");
    f.write("m.p01", b"Plan Title=File\nGeom File=g01\n");
    fs::create_dir_all(f.0.join("m.p02")).unwrap();
    fs::create_dir_all(f.0.join("m.g02")).unwrap();
    f.write("m.g01", b"geometry");
    let c = ProjectCatalog::open(project).unwrap();
    assert_eq!(c.plans[0].link.availability, Availability::Present);
    assert!(c.plans[0].plan.is_ok());
    assert_eq!(c.plans[1].link.availability, Availability::NotFile);
    assert_eq!(
        c.plans[1].plan.as_ref().unwrap_err().kind,
        CatalogErrorKind::Io
    );
    assert_eq!(c.plans[2].link.availability, Availability::Missing);
    assert_eq!(
        c.plans[2].plan.as_ref().unwrap_err().kind,
        CatalogErrorKind::NotFound
    );
    assert_eq!(c.geometries[0].availability, Availability::Present);
    assert_eq!(c.geometries[1].availability, Availability::NotFile);
    let absent = f.0.join("absent.prj");
    let e = ProjectCatalog::open(&absent).unwrap_err();
    assert_eq!((e.kind, e.path), (CatalogErrorKind::NotFound, absent));
    assert_eq!(
        Plan::open(f.0.join("absent.p01")).unwrap_err().kind,
        CatalogErrorKind::NotFound
    );
    assert_eq!(
        ProjectCatalog::open(&f.0).unwrap_err().kind,
        CatalogErrorKind::Io
    );
}

#[test]
fn parent_directory_escapes_are_reported_per_link() {
    // Escaping declarations retain inventory without opening the target.
    let f = Fixture::new();
    f.write("outside.p01", b"Plan Title=Outside\n");
    f.write("outside.g01", b"geometry");
    for decl in [
        "../outside.p01",
        "..\u{5c}outside.p01",
        "nested/../../outside.p01",
    ] {
        let project = f.write(
            "proj/inner.prj",
            format!("Proj Title=Esc\nPlan File={decl}\n").as_bytes(),
        );
        let c = ProjectCatalog::open(&project).unwrap();
        assert_eq!(c.plans[0].link.availability, Availability::OutsideProject);
        assert!(c.plans[0].plan.is_err());
        assert!(!c.diagnostics.is_empty());
    }
    // Inside the tree the same syntax is ordinary.
    let project = f.write("tree/a.prj", b"Proj Title=Ok\nPlan File=sub/../a.p01\n");
    f.write("tree/a.p01", b"Plan Title=A\n");
    assert_eq!(
        ProjectCatalog::open(project).unwrap().plans[0]
            .link
            .availability,
        Availability::Present
    );
}

#[test]
fn bug_parent_directory_escape_has_no_containment_status() {
    let f = Fixture::new();
    f.write("outside.p01", b"Plan Title=Outside\n");
    let project = f.write(
        "proj/inner.prj",
        b"Proj Title=Esc\nPlan File=../outside.p01\n",
    );
    let c = ProjectCatalog::open(&project);
    // Expected: an explicit rejection or diagnostic (or a non-Present availability status).
    let flagged = match &c {
        Err(_) => true,
        Ok(c) => !c.diagnostics.is_empty() || c.plans[0].link.availability != Availability::Present,
    };
    assert!(
        flagged,
        "escaping reference silently accepted: {}",
        c.is_ok()
    );
}

#[cfg(windows)]
#[test]
fn windows_absolute_and_drive_relative_references() {
    let f = Fixture::new();
    let target = f.write("elsewhere/abs.p01", b"Plan Title=Abs\n");
    let project = f.write(
        "proj/a.prj",
        format!("Proj Title=Abs\nPlan File={}\n", target.display()).as_bytes(),
    );
    let c = ProjectCatalog::open(project).unwrap();
    assert_eq!(c.plans[0].link.availability, Availability::OutsideProject);
    assert!(c.plans[0].plan.is_err());
}

#[cfg(not(windows))]
#[test]
fn foreign_windows_absolute_reference_is_malformed_on_posix() {
    let f = Fixture::new();
    let project = f.write("a.prj", b"Proj Title=Abs\nPlan File=C:/models/x.p01\n");
    assert_eq!(
        ProjectCatalog::open(project).unwrap().plans[0]
            .link
            .availability,
        Availability::OutsideProject
    );
}

#[test]
fn duplicate_plan_declarations_are_kept_in_order_without_a_diagnostic() {
    // Characterization of current behaviour for repeated plan numbers.
    let f = Fixture::new();
    let project = f.write("d.prj", b"Proj Title=Dup\nPlan File=p01\nPlan File=p02\nPlan File=p01\nPlan File=P01\nCurrent Plan=p02\n");
    f.write("d.p01", b"Plan Title=One\n");
    f.write("d.p02", b"Plan Title=Two\n");
    let c = ProjectCatalog::open(project).unwrap();
    let titles: Vec<_> = c
        .plans
        .iter()
        .map(|p| p.plan.as_ref().unwrap().title.clone())
        .collect();
    assert_eq!(
        titles,
        ["One", "Two", "One", "One"],
        "declaration order is preserved, repeats included"
    );
    assert_eq!(c.plans[0].link.path, c.plans[2].link.path);
    assert_eq!(c.plans[3].link.declared, "P01");
    assert!(
        c.plans[3].link.case_adjusted,
        "differently cased number resolves to the existing file"
    );
    assert_eq!(
        c.diagnostics
            .iter()
            .filter(|d| matches!(d, ProjectDiagnostic::DuplicatePlan { .. }))
            .count(),
        2
    );
}

#[test]
fn bug_duplicate_plan_numbers_have_no_explicit_status() {
    let f = Fixture::new();
    let project = f.write("d.prj", b"Proj Title=Dup\nPlan File=p01\nPlan File=p01\n");
    f.write("d.p01", b"Plan Title=One\n");
    let c = ProjectCatalog::open(project).unwrap();
    assert!(
        !c.diagnostics.is_empty(),
        "duplicate plan number reported: plans={}",
        c.plans.len()
    );
}

#[test]
fn current_plan_diagnostic_and_listed_variants() {
    let f = Fixture::new();
    f.write("c.p01", b"Plan Title=One\n");
    f.write("c.p02", b"Plan Title=Two\n");
    let unlisted = f.write("c1.prj", b"Proj Title=C\nCurrent Plan=p02\nPlan File=p01\n");
    f.write("c1.p01", b"Plan Title=One\n");
    f.write("c1.p02", b"Plan Title=Two\n");
    let c = ProjectCatalog::open(unlisted).unwrap();
    assert_eq!(
        c.diagnostics,
        vec![ProjectDiagnostic::CurrentPlanNotListed {
            declared: "p02".into()
        }]
    );
    assert_eq!(
        c.current_plan.as_ref().unwrap().availability,
        Availability::Present
    );
    // Same plan spelled with different case and a path prefix is recognised as listed.
    let listed = f.write(
        "c2.prj",
        b"Proj Title=C\nCurrent Plan=./C2.P01\nPlan File=c2.p01\n",
    );
    f.write("c2.p01", b"Plan Title=One\n");
    let c = ProjectCatalog::open(listed).unwrap();
    assert!(c.diagnostics.is_empty(), "{:?}", c.diagnostics);
    // A current plan with no plan list at all is a diagnostic, not an error.
    let none = f.write("c3.prj", b"Proj Title=C\nCurrent Plan=p01\n");
    let c = ProjectCatalog::open(none).unwrap();
    assert_eq!(c.diagnostics.len(), 1);
    assert_eq!(
        c.current_plan.as_ref().unwrap().availability,
        Availability::Missing
    );
    let conflict = f.write(
        "c4.prj",
        b"Proj Title=C\nCurrent Plan=p01\nCurrent Plan=p02\n",
    );
    assert_eq!(
        ProjectCatalog::open(conflict).unwrap_err().kind,
        CatalogErrorKind::Malformed
    );
}

#[test]
fn many_plans_keep_declaration_order_and_independent_statuses() {
    let f = Fixture::new();
    let mut body = String::from("Proj Title=Many\n");
    for n in 1..=120 {
        body.push_str(&format!("Plan File=p{n:02}\n"));
        match n % 3 {
            0 => {}
            1 => {
                f.write(
                    &format!("many.p{n:02}"),
                    format!("Plan Title=Plan {n}\n").as_bytes(),
                );
            }
            _ => {
                f.write(&format!("many.p{n:02}"), b"not a plan\n");
            }
        }
    }
    let c = ProjectCatalog::open(f.write("many.prj", body.as_bytes())).unwrap();
    assert_eq!(c.plans.len(), 120);
    for (i, entry) in c.plans.iter().enumerate() {
        let n = i + 1;
        assert!(entry.link.declared.ends_with(&format!("{n:02}")));
        match n % 3 {
            0 => assert_eq!(
                entry.plan.as_ref().unwrap_err().kind,
                CatalogErrorKind::NotFound
            ),
            1 => assert_eq!(entry.plan.as_ref().unwrap().title, format!("Plan {n}")),
            _ => assert_eq!(
                entry.plan.as_ref().unwrap_err().kind,
                CatalogErrorKind::Malformed
            ),
        }
    }
}

#[test]
fn flow_and_geometry_sources_are_classified_by_prefix() {
    let f = Fixture::new();
    let project = f.write("k.prj", b"Proj Title=K\nGeom File=g01\nGeom File=G02\nFlow File=f01\nUnsteady File=u01\nUnsteady File=u02\nQuasiUnsteady File=q01\n");
    let c = ProjectCatalog::open(project).unwrap();
    assert_eq!(c.geometries.len(), 2);
    assert_eq!(
        (
            c.steady_flows.len(),
            c.unsteady_flows.len(),
            c.quasi_unsteady_flows.len()
        ),
        (1, 2, 1)
    );
    for (flow, want) in [
        ("f01", FlowKind::Steady),
        ("U07", FlowKind::Unsteady),
        ("q12", FlowKind::QuasiUnsteady),
        ("sub/k.u03", FlowKind::Unsteady),
    ] {
        let p = plan(format!("Plan Title=T\nFlow File={flow}\n").as_bytes()).unwrap();
        assert_eq!(p.flow.unwrap().kind, want, "{flow}");
    }
    let p = plan(b"Plan Title=T\nGeom File=g03\n").unwrap();
    let hdf = p.geometry_hdf.unwrap();
    assert_eq!(hdf.declared, "derived: append .hdf");
    assert!(hdf.path.to_string_lossy().ends_with("probe.g03.hdf"));
}

fn sim(value: &str) -> Result<Plan, CatalogError> {
    plan(format!("Plan Title=T\nSimulation Date={value}\n").as_bytes())
}
fn interval(key: &str, value: &str) -> Result<Plan, CatalogError> {
    plan(format!("Plan Title=T\n{key}={value}\n").as_bytes())
}

#[test]
fn simulation_date_variants_parse_or_fail_explicitly() {
    let ok = [
        (
            "01JAN2020,0000,02JAN2020,2400",
            (2020, 1, 1, 0, 0),
            (2020, 1, 2, 24, 0),
        ),
        (
            "1jan2020,00:00,2Feb2020,23:59",
            (2020, 1, 1, 0, 0),
            (2020, 2, 2, 23, 59),
        ),
        (
            "29FEB2000,0000,01MAR2000,0000",
            (2000, 2, 29, 0, 0),
            (2000, 3, 1, 0, 0),
        ),
        (
            "31DEC9999,1200,01JAN12020,0100",
            (9999, 12, 31, 12, 0),
            (12020, 1, 1, 1, 0),
        ),
        (
            " 05MAY1999 , 0030 , 06MAY1999 , 2400 ",
            (1999, 5, 5, 0, 30),
            (1999, 5, 6, 24, 0),
        ),
    ];
    for (raw, a, b) in ok {
        let w = sim(raw).unwrap().simulation.unwrap();
        let got = |d: &DeclaredDateTime| (d.year, d.month, d.day, d.hour, d.minute);
        assert_eq!((got(&w.start), got(&w.end)), (a, b), "{raw}");
        assert_eq!(w.raw, raw.trim());
    }
    let w = sim("01JAN2020, 2400 ,02JAN2020,0000")
        .unwrap()
        .simulation
        .unwrap();
    assert_eq!(
        (w.start.raw_time.as_str(), w.start.hour),
        ("2400", 24),
        "raw declaration retained"
    );
    let bad = [
        "29FEB1900,0000,01MAR1900,0000",
        "31APR2020,0000,01MAY2020,0000",
        "00JAN2020,0000,01JAN2020,0000",
        "01JAN0000,0000,01JAN0001,0000",
        "01XXX2020,0000,01JAN2020,0000",
        "01JAN2020,2360,01JAN2020,0000",
        "01JAN2020,2401,01JAN2020,0000",
        "01JAN2020,0000,02JAN2020",
        "01JAN2020,0000,02JAN2020,0000,extra",
        "01JAN2020,000,02JAN2020,0000",
        "01JAN2020,00000,02JAN2020,0000",
        "01JAN2020,ABCD,02JAN2020,0000",
        "01JAN,0000,02JAN2020,0000",
        "JAN2020,0000,02JAN2020,0000",
        "123JAN2020,0000,02JAN2020,0000",
        "01JAN20-20,0000,02JAN2020,0000",
        "01JAN2020,0000,02JAN2020,-100",
        "\u{ff10}1JAN2020,0000,02JAN2020,0000",
        "01J\u{e1}N2020,0000,02JAN2020,0000",
    ];
    for raw in bad {
        assert_eq!(kind(sim(raw)), CatalogErrorKind::Malformed, "{raw:?}");
    }
}

#[test]
fn interval_variants_parse_or_fail_explicitly() {
    let ok = [
        ("1.5HOUR", 1.5, IntervalUnit::Hour),
        ("1 Hour", 1.0, IntervalUnit::Hour),
        ("30SEC", 30.0, IntervalUnit::Second),
        ("10min", 10.0, IntervalUnit::Minute),
        ("2Days", 2.0, IntervalUnit::Day),
        ("1WEEK", 1.0, IntervalUnit::Week),
        ("6MON", 6.0, IntervalUnit::Month),
        ("1YR", 1.0, IntervalUnit::Year),
        ("0.001SECONDS", 0.001, IntervalUnit::Second),
        ("  4HR  ", 4.0, IntervalUnit::Hour),
    ];
    for (raw, amount, unit) in ok {
        let p = interval("Output Interval", raw).unwrap();
        assert_eq!(p.intervals.len(), 1);
        assert_eq!(p.intervals[0].0, "Output Interval");
        assert_eq!(
            p.intervals[0].1.value,
            IntervalValue::Duration { amount, unit },
            "{raw}"
        );
    }
    let p = interval("Mapping Interval", "max profile").unwrap();
    assert_eq!(p.intervals[0].1.value, IntervalValue::MaximumProfile);
    let p = plan(b"Plan Title=T\nInstantaneous Interval=1HOUR\nMapping Interval=2HOUR\nOutput Interval=3HOUR\nComputation Interval=4SEC\n").unwrap();
    let order: Vec<_> = p.intervals.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(
        order,
        [
            "Computation Interval",
            "Output Interval",
            "Mapping Interval",
            "Instantaneous Interval"
        ]
    );
    for raw in [
        "0SEC",
        "-1HOUR",
        "0.0MIN",
        "1e3SEC",
        "1.5E2SEC",
        "SEC",
        "3",
        "",
        "INFSEC",
        "NaNSEC",
        "1FORTNIGHT",
        "1..5HOUR",
        "1,5HOUR",
        "1HOUR2",
    ] {
        let r = interval("Computation Interval", raw);
        if raw.is_empty() {
            // An empty value is an absent optional key, not an error.
            assert!(r.unwrap().intervals.is_empty());
        } else {
            assert_eq!(kind(r), CatalogErrorKind::Malformed, "{raw:?}");
        }
    }
}

#[test]
fn raw_source_text_is_retained_exactly_for_declared_values() {
    let raw = b"Plan Title=Exact\r\nSimulation Date=01JAN3000,0000,02JAN3000,2400\r\nOutput Interval=1 Hour\r\n";
    let p = plan(raw).unwrap();
    assert_eq!(p.source.bytes, raw);
    assert_eq!(
        p.simulation.as_ref().unwrap().raw,
        "01JAN3000,0000,02JAN3000,2400"
    );
    assert_eq!(p.intervals[0].1.raw, "1 Hour");
    assert_eq!(p.simulation.unwrap().start.raw_date, "01JAN3000");
}
