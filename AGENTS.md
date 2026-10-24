# AGENTS.md

Guidance for AI coding agents and human contributors working in this
repository. Read [README.md](README.md) and [docs/architecture.md](docs/architecture.md)
first.

## What this repository is

- `crates/ras-commander-rust`: the engine. A read-only Rust library for HEC-RAS
  project/plan text and the HDF5 files HEC-RAS writes. It must stay free of
  Python, PyO3 and NumPy, because other Rust consumers use it directly.
- `python/`: the `ras_commander_fast_hdf` extension (PyO3, maturin). It adapts
  the engine to Python and holds kernels that only ras-commander uses (mesh
  polygon assembly, fluvial/pluvial, PMTiles and vector exports, RAS text).
- `vendor/`: patched copies of `hdf5-reader` and `hdf5-core`. Change them only
  with a matching entry in `vendor/LOCAL_PATCHES.md` and a patch in
  `vendor/patches/`.
- ras-commander (a separate repository) owns the public Python API, the
  h5py reference path, the `auto | h5py | native` routing and the equivalence
  suites that qualify this engine.

## Rules that protect correctness

- **Exact or refuse.** A read either returns exactly what h5py would give
  ras-commander (values, dtype, shape, strings, attributes, order) or returns a
  typed error (`unsupported`, `corrupt`, `not_found`, or `unsupported` with
  code `resource_limit` when over budget). Never guess,
  silently repair, coerce, skip or turn unavailable data into an empty result.
  In `auto` mode ras-commander falls back to h5py on any refusal, so a refusal is
  always safe and a wrong value never is.
- **No panics on file input.** Validate counts, offsets, ranges, arithmetic and
  allocation sizes before using them; release builds keep `overflow-checks`.
  The bridge turns any panic that escapes into a Python exception
  (`python/src/panic_guard.rs`), but that is a backstop, not a design. The
  engine is `#![forbid(unsafe_code)]`.
- **Budget every read** before allocating. A read above the per-call budget is
  refused (ras-commander then reads it in chunks), never allocated blindly.
- **Never guess a CRS, unit or time zone.** Report the stored evidence.
- **Keep identities.** Mesh names, cell/face IDs, cross-section river/reach/
  station identities and time axes are carried explicitly, not by position.
- **Read-only.** Nothing here writes HEC-RAS files or runs HEC-RAS.

## Compatibility with ras-commander

- `python/src/lib.rs` defines `API_VERSION`. ras-commander uses the extension
  only when `API_VERSION` equals the value it was built against. Any change to
  a function signature, return shape, error class or behavior that
  ras-commander relies on needs a new `API_VERSION` and a matching
  ras-commander change. Additive changes that ras-commander does not call yet
  can keep the version.
- `ENGINE_COMMIT` is the commit of this repository the wheel was built from
  (`python/build.rs`). Release builds must come from a clean, tagged commit.
- Before a release, qualify the wheel with ras-commander's suites in all three
  modes; see [docs/testing.md](docs/testing.md).

## Working conventions

- Run before proposing a change:
  - `cargo fmt --all -- --check`
  - `cargo clippy --locked --all-features --all-targets --no-deps -- -D warnings`
  - `cargo test --locked --all-features` (engine) and
    `cargo test --locked --manifest-path python/Cargo.toml` (bridge)
  - `cargo test --locked --all-features --manifest-path vendor/hdf5-reader/Cargo.toml`
    when touching `vendor/`
- Engine tests use small committed fixtures under
  `crates/ras-commander-rust/tests/fixtures`. Generate new synthetic fixtures
  with a script committed next to them, and record their SHA-256 in the
  fixture manifest. Real HEC-RAS models are not committed; real-file tests read
  paths from environment variables and are skipped when unset.
- Only publicly available models (HEC-RAS example projects, public agency
  data) may be named in code, tests or docs. Never commit client models, client
  file paths, credentials or machine-specific paths.
- Regenerate the bundled license notices when dependencies change
  (`python/scripts/generate_third_party_notices.py`, see
  [docs/building.md](docs/building.md)); CI checks they are current.
- Update [CHANGELOG.md](CHANGELOG.md) and the relevant page in `docs/` with
  every behavior change. Write docs for an engineer who uses HEC-RAS: plain
  language, short sentences, exact terms.

## Documentation site

- The docs in `docs/` publish to https://rascommander.info/fasthdf/ on every push
  to `main`. The build infrastructure is in
  `CLB-Engineering-Corporation/ras-commander-docs`. A broken `mkdocs.yml` or a
  missing nav page fails the live build.
- The site build adds the shared cross-product theme (product switcher,
  `stylesheets/clb-branding.css`) into `docs/overrides/` at publish time. Keep
  `theme.custom_dir: docs/overrides` and the `extra_css` entry in `mkdocs.yml`,
  and do not copy the shared theme into this repository.
- User-facing guidance for ras-commander users stays on the ras-commander site
  (https://rascommander.info/ras/native-engine/); link to it rather than
  duplicating it here.
