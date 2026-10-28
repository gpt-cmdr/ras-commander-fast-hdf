# Releasing

## Versions

| Version | Where | Changes when |
|---|---|---|
| Package version (`0.2.0`) | `Cargo.toml` and `python/pyproject.toml` (must match; the release workflow checks) | Every release (semantic versioning; 0.x while the contract settles). The crate and the wheel share it. |
| Extension `API_VERSION` (11) | `src/python/mod.rs` | A change ras-commander must know about: a signature, return shape, error class or behavior it relies on. |
| Engine `API_VERSION` (10) | `src/api.rs` | A change to the Rust engine's public surface. |
| `TEXT_API_VERSION` (1) | `src/python/text_bindings.rs` | A change to the RAS text bindings. |

ras-commander accepts only the extension `API_VERSION` it was built for.
Raising it therefore needs a matching ras-commander release; publish the wheel
first, then the ras-commander version that requires it. A release that keeps
`API_VERSION` works with the ras-commander versions that already use it.

## Steps

1. Update `CHANGELOG.md` and the version in `Cargo.toml` and
   `python/pyproject.toml`.
2. Regenerate both licence notice files if `Cargo.lock` changed
   ([Building](building.md#licence-notices)).
3. Merge to `main` with CI green.
4. Qualify the CI-built wheels for Windows and Linux with ras-commander
   ([Testing](testing.md#qualifying-a-wheel-with-ras-commander)).
5. Tag the qualified commit `vX.Y.Z` and push the tag. The release workflow
   builds the wheels and source distribution from the tag, attaches them to a
   GitHub Release with SHA-256 checksums, and publishes them to PyPI.
6. Check that `pip install ras-commander-fast-hdf==X.Y.Z` works in a clean
   environment, and that `ENGINE_COMMIT` equals the tagged commit.
7. Publish the crate to crates.io (below).

Never publish a wheel built from a dirty tree; its `ENGINE_COMMIT` would not
identify the source.

## PyPI

The release workflow's PyPI job runs only when the repository variable
`PUBLISH_PYPI` is `true`. It uses PyPI trusted publishing (no stored token) once
the PyPI project trusts this repository's `release.yml` workflow and the `pypi`
environment. Until then, upload the files from the GitHub Release by hand:

```bash
python -m twine upload dist/*
```

## crates.io

Publishing the crate is a maintainer step; no workflow does it. From a clean
checkout of the release tag:

```bash
cargo publish --locked --dry-run
cargo publish --locked
```

CI runs the same dry run on every push. The package holds only what is needed
to build, plus `LICENSE`, `README.md`, `CHANGELOG.md`, `THIRD-PARTY-NOTICES.md`
and the licence and provenance files from `vendor/` (the `include` list
in `Cargo.toml`). Tests, fixtures and Python packaging are not shipped.
crates.io versions cannot be replaced, only yanked, so publish only a tag whose
wheels have been qualified. `ras-commander-fast-hdf` is the only crate this
repository publishes; the former engine crate name `ras-commander-rust` is
retired.
