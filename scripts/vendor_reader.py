"""Rebuild or verify the vendored HDF5 reader in vendor/.

vendor/ holds hdf5-reader and hdf5-core from roteiro-gis/netcdf-rust at tag
v0.9.3 with the local patch series vendor/patches/*.patch applied. This script
reproduces that tree from an upstream clone and records or checks the SHA-256
of every vendored file in vendor/PROVENANCE.json.

Usage:
  python scripts/vendor_reader.py --check
      Verify every file listed in vendor/PROVENANCE.json (no network, no Git).
  python scripts/vendor_reader.py --upstream PATH [--write]
      PATH is a clone of https://github.com/roteiro-gis/netcdf-rust that
      contains the v0.9.3 commit. Apply the patches to that commit in a
      temporary worktree, compare the result with vendor/ (or replace vendor/
      sources with it when --write is given) and rewrite PROVENANCE.json.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
VENDOR = ROOT / "vendor"
MANIFEST = VENDOR / "PROVENANCE.json"

UPSTREAM_URL = "https://github.com/roteiro-gis/netcdf-rust"
UPSTREAM_TAG = "v0.9.3"
UPSTREAM_COMMIT = "3d708fd40c6809f5d215bd2d9f15e9a05de84bd0"
CRATE_VERSION = "0.9.3"

# Upstream path -> vendor path. Directories are copied recursively.
COPIED = {
    "LICENSE-MIT": "LICENSE-MIT",
    "LICENSE-APACHE": "LICENSE-APACHE",
    "README.md": "README.md",
    "hdf5-core/src": "hdf5-core/src",
    "hdf5-reader/src": "hdf5-reader/src",
    "hdf5-reader/tests": "hdf5-reader/tests",
    "hdf5-reader/testdata": "hdf5-reader/testdata",
    "testdata/hdf5": "testdata/hdf5",
}
# Files in vendor/ that are written locally rather than copied from upstream.
LOCAL = {
    "PROVENANCE.json",
    "LOCAL_PATCHES.md",
    "hdf5-reader/PROVENANCE.md",
    "hdf5-core/Cargo.toml",
    "hdf5-reader/Cargo.toml",
    "hdf5-reader/Cargo.lock",
}
SKIP_DIRS = {"target", ".git", "__pycache__"}


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def tree_files(base: Path) -> dict[str, str]:
    out = {}
    for path in sorted(base.rglob("*")):
        rel = path.relative_to(base)
        if path.is_file() and not SKIP_DIRS.intersection(rel.parts):
            key = rel.as_posix()
            if key != "PROVENANCE.json":
                out[key] = sha256(path)
    return out


def tree_digest(files: dict[str, str]) -> str:
    text = "".join(f"{name} {digest}\n" for name, digest in sorted(files.items()))
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def git(*args: str, cwd: Path) -> str:
    return subprocess.run(
        ["git", *args], cwd=cwd, check=True, capture_output=True, text=True
    ).stdout


def patched_upstream(upstream: Path, work: Path) -> Path:
    """Check out the upstream commit in a temporary worktree and apply the patches."""
    tree = work / "netcdf-rust"
    git("worktree", "add", "--detach", str(tree), UPSTREAM_COMMIT, cwd=upstream)
    patches = sorted(str(p) for p in (VENDOR / "patches").glob("*.patch"))
    git("-c", "user.name=vendor", "-c", "user.email=vendor@localhost",
        "am", "--quiet", *patches, cwd=tree)
    return tree


def copied_files(tree: Path) -> dict[str, str]:
    out = {}
    for src, dst in COPIED.items():
        path = tree / src
        if path.is_file():
            out[dst] = sha256(path)
        else:
            for name, digest in tree_files(path).items():
                out[f"{dst}/{name}"] = digest
    return out


def upstream_diff(upstream_tree: Path, base_tree: Path) -> list[str]:
    changed = []
    for src in ("hdf5-core/src", "hdf5-reader/src"):
        new = tree_files(upstream_tree / src)
        old = tree_files(base_tree / src) if (base_tree / src).exists() else {}
        changed += [f"{src}/{k}" for k in sorted(new) if old.get(k) != new[k]]
    return changed


def write_manifest(patched: Path, base: Path) -> None:
    files = tree_files(VENDOR)
    patches = [
        {"patch": f"patches/{p.name}", "sha256": sha256(p)}
        for p in sorted((VENDOR / "patches").glob("*.patch"))
    ]
    manifest = {
        "upstream": UPSTREAM_URL,
        "upstream_tag": UPSTREAM_TAG,
        "base": UPSTREAM_COMMIT,
        "base_crate_version": CRATE_VERSION,
        "license": "MIT OR Apache-2.0 (both licence texts retained; copyright Ian Norden)",
        "description": (
            "Source copy of the hdf5-reader and hdf5-core crates from upstream tag "
            f"{UPSTREAM_TAG} with the local patch series in patches/ applied in order "
            "(git am). No crates.io copy of these crates is used."
        ),
        "change_notice": "hdf5-reader/PROVENANCE.md",
        "local_files": sorted(LOCAL - {"PROVENANCE.json"}),
        "patches": patches,
        "upstream_source_diff": upstream_diff(patched, base),
        "sha256_tree_definition": (
            "SHA256 of sorted UTF-8 relative-path + space + file SHA256 + LF, "
            "excluding PROVENANCE.json"
        ),
        "patched_tree_sha256": tree_digest(files),
        "files": files,
    }
    MANIFEST.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8", newline="\n")


def check() -> int:
    manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))
    recorded = manifest["files"]
    actual = tree_files(VENDOR)
    errors = [f"modified: {k}" for k in recorded if k in actual and actual[k] != recorded[k]]
    errors += [f"missing: {k}" for k in recorded if k not in actual]
    errors += [f"unrecorded: {k}" for k in actual if k not in recorded]
    if tree_digest(recorded) != manifest["patched_tree_sha256"]:
        errors.append("patched_tree_sha256 does not match the recorded file list")
    for entry in manifest["patches"]:
        if recorded.get(entry["patch"]) != entry["sha256"]:
            errors.append(f"patch hash mismatch: {entry['patch']}")
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print(f"vendor/ matches PROVENANCE.json: {len(recorded)} files, "
          f"upstream {manifest['upstream_tag']} ({manifest['base'][:7]}), "
          f"{len(manifest['patches'])} patches, tree {manifest['patched_tree_sha256']}")
    return 0


def regenerate(upstream: Path, write: bool) -> int:
    with tempfile.TemporaryDirectory() as tmp:
        work = Path(tmp)
        tree = patched_upstream(upstream, work)
        base = work / "base"
        git("worktree", "add", "--detach", str(base), UPSTREAM_COMMIT, cwd=upstream)
        try:
            expected = copied_files(tree)
            if write:
                for src, dst in COPIED.items():
                    target = VENDOR / dst
                    if target.is_dir():
                        shutil.rmtree(target)
                    target.parent.mkdir(parents=True, exist_ok=True)
                    if (tree / src).is_dir():
                        shutil.copytree(tree / src, target)
                    else:
                        shutil.copyfile(tree / src, target)
            actual = tree_files(VENDOR)
            mismatched = [k for k, v in expected.items() if actual.get(k) != v]
            stray = [
                k for k in actual
                if k not in expected and k not in LOCAL and not k.startswith("patches/")
            ]
            if mismatched or stray:
                for k in mismatched:
                    print(f"differs from upstream + patches: {k}", file=sys.stderr)
                for k in stray:
                    print(f"not from upstream and not a local file: {k}", file=sys.stderr)
                return 1
            write_manifest(tree, base)
        finally:
            git("worktree", "remove", "--force", str(tree), cwd=upstream)
            git("worktree", "remove", "--force", str(base), cwd=upstream)
    print(f"vendor/ reproduced from {UPSTREAM_TAG} + patches; PROVENANCE.json written")
    return check()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--upstream", type=Path)
    parser.add_argument("--write", action="store_true")
    args = parser.parse_args()
    if args.upstream:
        return regenerate(args.upstream.resolve(), args.write)
    return check()


if __name__ == "__main__":
    sys.exit(main())
