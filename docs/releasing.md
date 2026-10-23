# Releasing

## Versions

| Version | Where | Changes when |
|---|---|---|
| Package version (`0.1.0`) | `python/Cargo.toml` and `python/pyproject.toml` | Every release (semantic versioning; 0.x while the contract settles). |
| Extension `API_VERSION` (11) | `python/src/lib.rs` | A change ras-commander must know about: a signature, return shape, error class or behavior it relies on. |
| Engine `API_VERSION` (10) | `crates/ras-commander-rust/src/api.rs` | A change to the Rust engine's public surface. |
| `TEXT_API_VERSION` (1) | `python/src/text_bindings.rs` | A change to the RAS text bindings. |

ras-commander accepts only the extension `API_VERSION` it was built for.
Raising it therefore needs a matching ras-commander release; publish the wheel
first, then the ras-commander version that requires it. A release that keeps
`API_VERSION` works with the ras-commander versions that already use it.

## Steps

1. Update `CHANGELOG.md` and the version in `python/Cargo.toml` and
   `python/pyproject.toml`.
2. Regenerate the license notices if `Cargo.lock` changed
   ([Building](building.md#license-notices)).
3. Merge to `main` with CI green.
4. Qualify the CI-built wheels for Windows and Linux with ras-commander
   ([Testing](testing.md#qualifying-a-wheel-with-ras-commander)).
5. Tag the qualified commit `vX.Y.Z` and push the tag. The release workflow
   builds the wheels and source distribution from the tag, attaches them to a
   GitHub Release with SHA-256 checksums, and publishes them to PyPI.
6. Check that `pip install ras-commander-fast-hdf==X.Y.Z` works in a clean
   environment, and that `ENGINE_COMMIT` equals the tagged commit.

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
