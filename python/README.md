# ras-commander-fast-hdf

A native (Rust) reader for HEC-RAS HDF files, used by
[ras-commander](https://github.com/gpt-cmdr/ras-commander).

Install it next to ras-commander and the `Hdf*` classes read plan results,
geometry, terrain and land-cover HDF files through this engine instead of
h5py. Results are identical (values, dtypes, columns, index, CRS, attributes
and exceptions). Large reads are faster: 2D cell polygons 4–19× depending on
mesh size, and mesh time series 2–7× on large meshes. Small reads are a few
milliseconds slower.

## Install

```bash
pip install ras-commander ras-commander-fast-hdf
```

```python
from ras_commander import fast_hdf
fast_hdf.available()   # True
```

Wheels: Windows x64, Linux x64 (`manylinux_2_28`) and macOS arm64, CPython
3.10 or later. No HDF5 C library or Rust toolchain is needed to install.
Windows and Linux are qualified against HEC-RAS result files; macOS builds
and passes the smoke tests but has not been qualified.

## Engine modes

Choose the mode with `RAS_COMMANDER_FAST_HDF`:

| Value | Behavior |
|---|---|
| `auto` (default) | Native where it is qualified; h5py whenever the engine refuses a read. |
| `h5py` | Never use the native engine. |
| `native` | Native or raise `FastHdfError`; for testing and qualification. |

## Links

- User guide: <https://rascommander.info/ras/native-engine/>
- Source, developer docs and issues: <https://github.com/gpt-cmdr/ras-commander-fast-hdf>
- Changelog: <https://github.com/gpt-cmdr/ras-commander-fast-hdf/blob/main/CHANGELOG.md>

## License

MIT license. The wheel includes the licenses of every bundled crate in
`THIRD-PARTY-NOTICES.md`.

ras-commander-fast-hdf is an independent open source project. HEC-RAS is
developed by the U.S. Army Corps of Engineers Hydrologic Engineering Center
(HEC). This project is not affiliated with, endorsed by, or supported by HEC
or USACE.
