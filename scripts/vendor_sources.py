"""Rebuild or verify the third-party source carried in vendor/.

ras-commander-fast-hdf compiles two third-party code bases as internal modules
of its own crate (see lib.rs):

* hdf5: the hdf5-reader and hdf5-core crates of roteiro-gis/netcdf-rust at tag
  v0.9.3, with the patch series vendor/patches/*.patch applied (git am).
  Modules crate::hdf5_reader and crate::hdf5_core.
* freestiler: the freestiler-core crate of walkerke/freestiler at commit
  beb64b0. Module crate::freestiler_core.

Each Rust source file is embedded with a small, mechanical transform (below):
a two-line notice is prepended and crate-relative paths are re-rooted at the
module. Everything else (licences, upstream tests and fixtures) is copied
byte for byte. vendor/PROVENANCE.json records, for every vendored file, its
SHA-256 and, for transformed files, the SHA-256 of the upstream (patched)
file it was derived from.

Usage:
  python scripts/vendor_sources.py --check
      Verify vendor/ against PROVENANCE.json (offline).
  python scripts/vendor_sources.py --netcdf-rust PATH --freestiler PATH [--write]
      PATH are clones of https://github.com/roteiro-gis/netcdf-rust and
      https://github.com/walkerke/freestiler containing the pinned commits.
      Rebuild the vendored files from upstream, compare them with vendor/
      (replace them with --write) and rewrite PROVENANCE.json.
"""
from __future__ import annotations

import argparse
import hashlib
import io
import json
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
VENDOR = ROOT / "vendor"
MANIFEST = VENDOR / "PROVENANCE.json"
SKIP_DIRS = {"target", ".git", "__pycache__"}

# Files in vendor/ written locally (not derived from upstream).
LOCAL = [
    "LOCAL_PATCHES.md",
    "hdf5-reader/PROVENANCE.md",
    "freestiler-core/PROVENANCE.md",
    "reader-tests/Cargo.toml",
    "reader-tests/Cargo.lock",
    "reader-tests/src/lib.rs",
]

CRATE_PATH = rb"(?<![\w$:])crate::"


def notice(text: str) -> bytes:
    return text.encode("utf-8")


COMPONENTS = {
    "hdf5": {
        "upstream": "https://github.com/roteiro-gis/netcdf-rust",
        "tag": "v0.9.3",
        "commit": "3d708fd40c6809f5d215bd2d9f15e9a05de84bd0",
        "crates": {"hdf5-reader": "0.9.3", "hdf5-core": "0.9.3"},
        "license": "MIT OR Apache-2.0",
        "copyright": "Copyright (c) 2026 Ian Norden",
        "licence_files": ["LICENSE-MIT", "LICENSE-APACHE"],
        "change_notice": "hdf5-reader/PROVENANCE.md",
        "patches": "patches",
        # upstream path -> vendor path, copied byte for byte
        "copy": {
            "LICENSE-MIT": "LICENSE-MIT",
            "LICENSE-APACHE": "LICENSE-APACHE",
            "README.md": "README.md",
            "hdf5-reader/tests": "hdf5-reader/tests",
            "hdf5-reader/testdata": "hdf5-reader/testdata",
            "testdata/hdf5": "testdata/hdf5",
        },
        # upstream source dir -> (vendor dir, module, rules)
        "embed": {
            "hdf5-core/src": ("hdf5-core/src", "crate::hdf5_core", [
                (CRATE_PATH, rb"crate::hdf5_core::"),
            ]),
            "hdf5-reader/src": ("hdf5-reader/src", "crate::hdf5_reader", [
                (CRATE_PATH, rb"crate::hdf5_reader::"),
                (rb"(?<![\w:])hdf5_core::", rb"crate::hdf5_core::"),
                (rb'feature = "(?:rayon|lz4)"', rb'feature = "hdf"'),
            ]),
        },
        "omit": [],
        "notice": (
            "// ras-commander-fast-hdf: modified copy of roteiro-gis/netcdf-rust v0.9.3, MIT OR Apache-2.0,\n"
            "// Copyright (c) 2026 Ian Norden. Changes are listed in vendor/hdf5-reader/PROVENANCE.md.\n"
        ),
    },
    "freestiler": {
        "upstream": "https://github.com/walkerke/freestiler",
        "tag": None,
        "commit": "beb64b0d4414da895ec67d02b2f91db6ab6acc37",
        "crates": {"freestiler-core": "0.1.0"},
        "license": "MIT AND ISC",
        "copyright": (
            "Copyright (c) 2026 Kyle Walker (MIT); supercluster/: Supercluster 8.0.1 "
            "Copyright (c) 2021 Mapbox and KDBush 4.1.0 Copyright (c) 2026 Vladimir Agafonkin (ISC)"
        ),
        "licence_files": [
            "freestiler-core/LICENSE.md",
            "freestiler-core/LICENSE",
            "freestiler-core/LICENSE.note",
            "freestiler-core/COPYRIGHTS",
            "freestiler-core/src/supercluster/UPSTREAM-LICENSES",
        ],
        "change_notice": "freestiler-core/PROVENANCE.md",
        "patches": None,
        "copy": {
            "LICENSE.md": "freestiler-core/LICENSE.md",
            "LICENSE": "freestiler-core/LICENSE",
            "LICENSE.note": "freestiler-core/LICENSE.note",
            "inst/COPYRIGHTS": "freestiler-core/COPYRIGHTS",
            "src/rust/freestiler-core/src/supercluster/UPSTREAM-LICENSES":
                "freestiler-core/src/supercluster/UPSTREAM-LICENSES",
            "src/rust/freestiler-core/tests/fixtures": "freestiler-core/tests/fixtures",
        },
        "embed": {
            "src/rust/freestiler-core/src": ("freestiler-core/src", "crate::freestiler_core", [
                (CRATE_PATH, rb"crate::freestiler_core::"),
                (rb'feature = "(duckdb|fastpfor|fsst|geoparquet)"',
                 rb'any(/* upstream feature "\1" is not vendored */)'),
                # Unit tests that decode with upstream's dev-dependency mlt-core
                # (which builds C++ through CMake) are compiled out.
                (rb'(\n[ \t]*)#\[test\](\r?\n[ \t]*fn test_mlt_core_)',
                 rb'\1#[cfg(any(/* upstream test needs the dev-dependency mlt-core, not vendored */))]'
                 rb'\1#[test]\2'),
            ]),
        },
        # Only compiled with upstream features that are not vendored.
        "omit": [
            "src/rust/freestiler-core/src/file_input.rs",
            "src/rust/freestiler-core/src/streaming.rs",
        ],
        "notice": (
            "// ras-commander-fast-hdf: modified copy of walkerke/freestiler beb64b0 (freestiler-core), MIT;\n"
            "// supercluster/ is ISC. Changes are listed in vendor/freestiler-core/PROVENANCE.md.\n"
        ),
    },
}


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def tree_files(base: Path) -> dict[str, str]:
    out = {}
    for path in sorted(base.rglob("*")):
        rel = path.relative_to(base)
        if path.is_file() and not SKIP_DIRS.intersection(rel.parts):
            key = rel.as_posix()
            if key != "PROVENANCE.json":
                out[key] = sha256(path.read_bytes())
    return out


def tree_digest(files: dict[str, str]) -> str:
    text = "".join(f"{name} {digest}\n" for name, digest in sorted(files.items()))
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def embed(data: bytes, header: str, rules) -> bytes:
    newline = b"\r\n" if b"\r\n" in data else b"\n"
    for pattern, replacement in rules:
        data = re.sub(pattern, replacement, data)
    return header.encode("utf-8").replace(b"\n", newline) + data


def git(*args: str, cwd: Path, binary: bool = False):
    out = subprocess.run(["git", *args], cwd=cwd, check=True, capture_output=True)
    return out.stdout if binary else out.stdout.decode("utf-8")


def checkout(name: str, spec: dict, clone: Path, work: Path) -> Path:
    """Upstream tree at the pinned commit, with the patch series applied."""
    tree = work / name
    if spec["patches"]:
        git("worktree", "add", "--detach", str(tree), spec["commit"], cwd=clone)
        patches = sorted(str(p) for p in (VENDOR / spec["patches"]).glob("*.patch"))
        git("-c", "user.name=vendor", "-c", "user.email=vendor@localhost",
            "am", "--quiet", *patches, cwd=tree)
    else:
        tree.mkdir()
        archive = git("archive", "--format=tar", spec["commit"], cwd=clone, binary=True)
        with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
            tar.extractall(tree, filter="data")
    return tree


def build(name: str, spec: dict, tree: Path) -> tuple[dict[str, bytes], dict[str, str]]:
    """Vendored files (vendor path -> bytes) and upstream hashes of embedded files."""
    files: dict[str, bytes] = {}
    upstream: dict[str, str] = {}
    omit = set(spec["omit"])
    for src, dst in spec["copy"].items():
        path = tree / src
        if path.is_file():
            files[dst] = path.read_bytes()
        else:
            for item in sorted(path.rglob("*")):
                if item.is_file():
                    files[f"{dst}/{item.relative_to(path).as_posix()}"] = item.read_bytes()
    for src, (dst, _module, rules) in spec["embed"].items():
        for item in sorted((tree / src).rglob("*.rs")):
            rel = item.relative_to(tree).as_posix()
            if rel in omit:
                continue
            key = f"{dst}/{item.relative_to(tree / src).as_posix()}"
            data = item.read_bytes()
            upstream[key] = sha256(data)
            files[key] = embed(data, spec["notice"], rules)
    return files, upstream


def describe(spec: dict) -> dict:
    out = {k: spec[k] for k in ("upstream", "tag", "commit", "crates", "license",
                                "copyright", "licence_files", "change_notice")}
    if spec["patches"]:
        out["patches"] = [
            {"patch": f"{spec['patches']}/{p.name}", "sha256": sha256(p.read_bytes())}
            for p in sorted((VENDOR / spec["patches"]).glob("*.patch"))
        ]
    out["omitted"] = spec["omit"]
    out["embedding"] = {
        "modules": {dst: module for dst, module, _ in spec["embed"].values()},
        "notice_header": spec["notice"],
        "rules": [
            {"pattern": p.decode(), "replacement": r.decode()}
            for _, _, rules in spec["embed"].values() for p, r in rules
        ],
    }
    return out


def write_manifest(upstream: dict[str, str]) -> None:
    files = tree_files(VENDOR)
    manifest = {
        "description": (
            "Third-party source compiled as internal modules of ras-commander-fast-hdf. "
            "Rust sources are embedded with the recorded mechanical rules (notice header, "
            "module-rooted paths); all other files are byte-for-byte upstream copies. "
            "Rebuild and compare with scripts/vendor_sources.py."
        ),
        "components": {name: describe(spec) for name, spec in COMPONENTS.items()},
        "local_files": LOCAL,
        "sha256_tree_definition": (
            "SHA256 of sorted UTF-8 relative-path + space + file SHA256 + LF, "
            "excluding PROVENANCE.json"
        ),
        "tree_sha256": tree_digest(files),
        "files": files,
        "upstream_sha256": dict(sorted(upstream.items())),
    }
    MANIFEST.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8", newline="\n")


def check() -> int:
    manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))
    recorded = manifest["files"]
    actual = tree_files(VENDOR)
    errors = [f"modified: {k}" for k in recorded if k in actual and actual[k] != recorded[k]]
    errors += [f"missing: {k}" for k in recorded if k not in actual]
    errors += [f"unrecorded: {k}" for k in actual if k not in recorded]
    if tree_digest(recorded) != manifest["tree_sha256"]:
        errors.append("tree_sha256 does not match the recorded file list")
    for spec in manifest["components"].values():
        for entry in spec.get("patches", []):
            if recorded.get(entry["patch"]) != entry["sha256"]:
                errors.append(f"patch hash mismatch: {entry['patch']}")
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    parts = ", ".join(
        f"{name} {spec['tag'] or spec['commit'][:7]}" for name, spec in manifest["components"].items()
    )
    print(f"vendor/ matches PROVENANCE.json: {len(recorded)} files ({parts}), "
          f"tree {manifest['tree_sha256']}")
    return 0


def regenerate(clones: dict[str, Path], write: bool) -> int:
    expected: dict[str, bytes] = {}
    upstream: dict[str, str] = {}
    with tempfile.TemporaryDirectory() as tmp:
        work = Path(tmp)
        worktrees = []
        try:
            for name, spec in COMPONENTS.items():
                tree = checkout(name, spec, clones[name], work)
                if spec["patches"]:
                    worktrees.append((clones[name], tree))
                files, hashes = build(name, spec, tree)
                expected.update(files)
                upstream.update(hashes)
        finally:
            for clone, tree in worktrees:
                git("worktree", "remove", "--force", str(tree), cwd=clone)
    if write:
        derived_roots = {
            dst for spec in COMPONENTS.values() for dst in spec["copy"].values()
        } | {dst for spec in COMPONENTS.values() for dst, _, _ in spec["embed"].values()}
        for root in derived_roots:
            target = VENDOR / root
            if target.is_dir():
                shutil.rmtree(target)
            elif target.is_file():
                target.unlink()
        for key, data in expected.items():
            path = VENDOR / key
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
    actual = tree_files(VENDOR)
    errors = [f"differs from upstream: {k}" for k, v in expected.items() if actual.get(k) != sha256(v)]
    errors += [
        f"neither upstream-derived nor a local file: {k}" for k in actual
        if k not in expected and k not in LOCAL and not k.startswith("patches/")
    ]
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    write_manifest(upstream)
    print("vendor/ reproduced from upstream; PROVENANCE.json written")
    return check()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--netcdf-rust", type=Path)
    parser.add_argument("--freestiler", type=Path)
    parser.add_argument("--write", action="store_true")
    args = parser.parse_args()
    if args.netcdf_rust or args.freestiler:
        if not (args.netcdf_rust and args.freestiler):
            parser.error("--netcdf-rust and --freestiler are both required")
        return regenerate(
            {"hdf5": args.netcdf_rust.resolve(), "freestiler": args.freestiler.resolve()},
            args.write,
        )
    return check()


if __name__ == "__main__":
    sys.exit(main())
