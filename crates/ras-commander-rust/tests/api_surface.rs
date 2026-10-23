#![cfg(all(feature = "hdf", feature = "mesh", feature = "results"))]
//! The machine-readable entry-point list must match the public session surface in the source.
use ras_commander_rust::{capabilities, API_VERSION};
use std::collections::BTreeSet;

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || c == char::from(95u8)
}

/// Public methods of every `impl ... ReadOnlyHdfSession` block, excluding deprecated aliases.
fn session_methods() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut stack = vec![std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src")];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|x| x != "rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path)
                .unwrap()
                .replace("\r\n", "\n");
            let (mut inside, mut deprecated) = (false, false);
            for line in text.lines() {
                if line.starts_with("impl") && line.ends_with("ReadOnlyHdfSession {") {
                    inside = true;
                } else if inside && line == "}" {
                    inside = false;
                } else if inside {
                    let t = line.trim_start();
                    if t.starts_with("#[deprecated") {
                        deprecated = true;
                    } else if let Some(rest) = t.strip_prefix("pub fn ") {
                        let name: String = rest.chars().take_while(|c| is_name_char(*c)).collect();
                        if !deprecated {
                            out.insert(name);
                        }
                        deprecated = false;
                    } else if !t.starts_with("#[") && !t.starts_with("///") {
                        deprecated = false;
                    }
                }
            }
        }
    }
    out
}

#[test]
fn entrypoint_list_matches_the_public_session_surface() {
    let listed: BTreeSet<String> = ras_commander_rust::api::ENTRYPOINTS
        .iter()
        .filter(|e| e.owner == "ReadOnlyHdfSession")
        .map(|e| e.name.to_owned())
        .collect();
    assert_eq!(listed, session_methods());
}

#[test]
fn capabilities_follow_compiled_features() {
    let c = capabilities();
    assert_eq!(c.api_version, API_VERSION);
    for f in ["catalog", "hdf", "mesh", "results"] {
        assert!(c.features.contains(&f), "{f}");
    }
    assert_eq!(
        c.entrypoints.len(),
        ras_commander_rust::api::ENTRYPOINTS.len()
    );
}
