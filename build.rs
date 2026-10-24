//! Build metadata for the Python extension: `ENGINE_COMMIT` and `ENGINE_FEATURES`.
//!
//! Only the `python` feature uses it; other builds do nothing here.
//!
//! The engine and the extension are one crate, so the engine commit is the
//! commit of this checkout. `RAS_ENGINE_COMMIT` overrides it (release builds
//! from a source archive). Git is asked only when this package directory is
//! itself a Git checkout, so a copy of the package inside some other
//! repository never reports that repository's commit. Without Git or the
//! override it is `"unknown"`, which ras-commander reports but accepts.
//!
//! `ENGINE_FEATURES` lists the enabled engine features in a fixed order
//! (`catalog`, `hdf`, `mesh`, `results`); the `python` feature enables all four.

use std::path::Path;
use std::process::Command;

const ENGINE_FEATURES: [&str; 4] = ["catalog", "hdf", "mesh", "results"];

fn engine_features() -> String {
    ENGINE_FEATURES
        .iter()
        .filter(|f| std::env::var_os(format!("CARGO_FEATURE_{}", f.to_ascii_uppercase())).is_some())
        .copied()
        .collect::<Vec<_>>()
        .join(",")
}

fn is_commit(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn engine_commit() -> Result<String, String> {
    if let Ok(value) = std::env::var("RAS_ENGINE_COMMIT") {
        let value = value.trim().to_ascii_lowercase();
        if !is_commit(&value) {
            return Err("RAS_ENGINE_COMMIT must be a complete 40-digit hexadecimal commit".into());
        }
        return Ok(value);
    }
    if !Path::new(".git").exists() {
        return Ok("unknown".into());
    }
    let output = Command::new("git").args(["rev-parse", "HEAD"]).output();
    match output {
        Ok(out) if out.status.success() => {
            let value = String::from_utf8_lossy(&out.stdout)
                .trim()
                .to_ascii_lowercase();
            Ok(if is_commit(&value) {
                value
            } else {
                "unknown".into()
            })
        }
        _ => Ok("unknown".into()),
    }
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=RAS_ENGINE_COMMIT");
    if std::env::var_os("CARGO_FEATURE_PYTHON").is_none() {
        return;
    }
    for head in [".git/HEAD", ".git/refs/heads"] {
        if Path::new(head).exists() {
            println!("cargo:rerun-if-changed={head}");
        }
    }
    match engine_commit() {
        Ok(commit) => {
            println!("cargo:rustc-env=RAS_ENGINE_COMMIT={commit}");
            println!("cargo:rustc-env=RAS_ENGINE_FEATURES={}", engine_features());
        }
        Err(message) => {
            eprintln!("python extension build metadata: {message}");
            std::process::exit(1);
        }
    }
}
