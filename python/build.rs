//! Build metadata exposed as `ENGINE_COMMIT` and `ENGINE_FEATURES`.
//!
//! The engine is built from this repository (`crates/ras-commander-rust`), so
//! the engine commit is the commit of this checkout. `RAS_ENGINE_COMMIT`
//! overrides it (release builds from a source archive); without Git or the
//! override it is `"unknown"`, which ras-commander reports but accepts.

use std::path::Path;
use std::process::Command;

fn engine_features() -> Result<String, String> {
    let text = std::fs::read_to_string("Cargo.toml")
        .map_err(|e| format!("cannot read Cargo.toml: {e}"))?;
    let manifest: toml::Value = text
        .parse()
        .map_err(|e| format!("invalid Cargo.toml: {e}"))?;
    let dep = manifest
        .get("dependencies")
        .and_then(|d| d.get("ras-commander-rust"))
        .ok_or("missing dependencies.ras-commander-rust")?;
    if dep.get("path").and_then(toml::Value::as_str) != Some("../crates/ras-commander-rust") {
        return Err(
            "the engine must be the in-repository crate ../crates/ras-commander-rust".into(),
        );
    }
    Ok(dep
        .get("features")
        .and_then(toml::Value::as_array)
        .ok_or("engine dependency must declare a features array")?
        .iter()
        .map(|v| v.as_str().ok_or("engine features must be strings"))
        .collect::<Result<Vec<_>, _>>()?
        .join(","))
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
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-env-changed=RAS_ENGINE_COMMIT");
    for head in ["../.git/HEAD", "../.git/refs/heads"] {
        if Path::new(head).exists() {
            println!("cargo:rerun-if-changed={head}");
        }
    }
    match (engine_commit(), engine_features()) {
        (Ok(commit), Ok(features)) => {
            println!("cargo:rustc-env=RAS_ENGINE_COMMIT={commit}");
            println!("cargo:rustc-env=RAS_ENGINE_FEATURES={features}");
        }
        (Err(message), _) | (_, Err(message)) => {
            eprintln!("bridge build metadata: {message}");
            std::process::exit(1);
        }
    }
}
