# ras-commander-fast-hdf

A native (Rust) reader for HEC-RAS HDF files, used by
[ras-commander](https://github.com/gpt-cmdr/ras-commander).

Install it next to ras-commander and the `Hdf*` classes read plan results,
geometry, terrain and land-cover HDF files through this engine instead of
h5py. Results are identical (values, dtypes, columns, index, CRS, attributes
and exceptions); large reads are faster, for example 2D cell polygons about
12× and mesh time series 3–5×.

```bash
pip install ras-commander ras-commander-fast-hdf
```

```python
from ras_commander import fast_hdf
fast_hdf.available()   # True
```

Choose the mode with `RAS_COMMANDER_FAST_HDF`:

| Value | Behaviour |
|---|---|
| `auto` (default) | Native where it is qualified; h5py whenever the engine refuses a read. |
| `h5py` | Never use the native engine. |
| `native` | Native or raise `FastHdfError`; for testing and qualification. |

Wheels: Windows x64, Linux x64 (`manylinux_2_28`) and macOS arm64, CPython
3.10 or later. No HDF5 C library or Rust toolchain is needed to install.

- User guide: <https://rascommander.info/ras/native-engine/>
- Source, developer docs and issues: <https://github.com/gpt-cmdr/ras-commander-fast-hdf>

MIT licence. The wheel includes the licences of every bundled crate in
`THIRD-PARTY-NOTICES.md`.
