"""Synthetic fixtures for the engine's two reader policies (src/hdf/reader.rs).

filter_*.h5: one 2-D f64 dataset "data" (shape 6 x 8, chunks 2 x 4, values
row * 8 + col) whose two-filter pipeline is written by libhdf5 with shuffle
and DEFLATE, then has the shuffle filter id patched in the (version 1) filter
pipeline message. Version 1 messages always store the name length, so the
message stays well formed. The chunk bytes are not changed.

- filter_lz4_then_deflate.h5: pipeline [LZ4 (32004), DEFLATE]. LZ4 has no
  qualified encoded-size bound and is an intermediate decoding stage.
- filter_unknown_then_deflate.h5: pipeline [40000 (unregistered), DEFLATE].
- filter_deflate_then_lz4.h5: pipeline [DEFLATE, LZ4]. LZ4 is the first
  decoding stage and is bounded by the chunk size; the policy does not refuse
  it (the chunk is not LZ4 data, so the read fails as corrupt).

ea_inline.h5: h5py libver="latest", "data" f64 shape (3, 5), maxshape
(None, 5), chunks (1, 5), values row * 5 + col: an extensible-array chunk
index whose 3 elements fit in its index block (4 by default), which the
engine reads.

ea_latest.h5 (not generated here): copied from the hdf5-reader 0.9.3 tree the
engine vendored before 0.3 (tests/fixtures/perf2/ea_latest.h5 of patch 0014).
h5py libver="latest", "data" f32 shape (50, 101), maxshape (None, 101),
chunks (1, 101), values row * 101 + col + 1: an extensible-array chunk index
with 50 elements, more than its index block holds.

ea_deep.h5: h5py libver="latest", "data" f32 shape (9000, 2), maxshape
(None, 2), chunks (1, 2), values row * 2 + col + 1: 9000 chunks, so the index
has super blocks and paged data blocks.

Run: python tests/fixtures/reader_policy/make_fixtures.py
"""
from pathlib import Path
import hashlib
import json

import h5py
import numpy as np

OUT = Path(__file__).resolve().parent
SHUFFLE = 2
DEFLATE = 1
LZ4 = 32004
UNKNOWN = 40000


def write(path: Path, first: int, second: int) -> None:
    """Write "data" with the pipeline [first, second] (shuffle or DEFLATE)."""
    data = np.arange(48, dtype="<f8").reshape(6, 8)
    with h5py.File(path, "w", libver="earliest") as f:
        dcpl = h5py.h5p.create(h5py.h5p.DATASET_CREATE)
        dcpl.set_chunk((2, 4))
        for filter_id in (first, second):
            if filter_id == SHUFFLE:
                dcpl.set_shuffle()
            else:
                dcpl.set_deflate(4)
        space = h5py.h5s.create_simple(data.shape)
        dset = h5py.h5d.create(f.id, b"data", h5py.h5t.IEEE_F64LE, space, dcpl=dcpl)
        dset.write(h5py.h5s.ALL, h5py.h5s.ALL, data)


def patch_shuffle(path: Path, first: int, second: int, new_id: int) -> None:
    """Replace the shuffle id in the file's only version 1 filter pipeline message."""
    raw = bytearray(path.read_bytes())
    # Version 1, two filters, six reserved bytes, then the first filter id.
    head = bytes([1, 2, 0, 0, 0, 0, 0, 0])
    at = raw.find(head + first.to_bytes(2, "little"))
    assert at >= 0 and raw.find(head, at + 1) < 0, "expected one v1 pipeline message"
    # Filter descriptions: id, name length, flags, client value count, name
    # (padded to 8), client values (4 bytes each, padded to 8 bytes).
    pos = at + len(head)
    for index, expected in enumerate((first, second)):
        filter_id = int.from_bytes(raw[pos:pos + 2], "little")
        assert filter_id == expected, (filter_id, expected)
        name_len = int.from_bytes(raw[pos + 2:pos + 4], "little")
        values = int.from_bytes(raw[pos + 6:pos + 8], "little")
        if filter_id == SHUFFLE:
            raw[pos:pos + 2] = new_id.to_bytes(2, "little")
        pos += 8 + (name_len + 7) // 8 * 8 + 4 * values + (4 if values % 2 else 0)
    path.write_bytes(bytes(raw))


def main() -> None:
    cases = {
        "filter_lz4_then_deflate.h5": (SHUFFLE, DEFLATE, LZ4),
        "filter_unknown_then_deflate.h5": (SHUFFLE, DEFLATE, UNKNOWN),
        "filter_deflate_then_lz4.h5": (DEFLATE, SHUFFLE, LZ4),
    }
    for name, (first, second, new_id) in cases.items():
        write(OUT / name, first, second)
        patch_shuffle(OUT / name, first, second, new_id)
    with h5py.File(OUT / "ea_inline.h5", "w", libver="latest") as f:
        f.create_dataset(
            "data",
            data=np.arange(15, dtype="<f8").reshape(3, 5),
            maxshape=(None, 5),
            chunks=(1, 5),
        )
    # A deep extensible-array chunk index: 9000 chunks need super blocks, data
    # blocks and paged data blocks. h5py (libhdf5) is the oracle: its read of
    # every value is checked here and recorded as a SHA-256 in the manifest.
    deep = (np.arange(9000 * 2, dtype="<f4").reshape(9000, 2) + 1)
    with h5py.File(OUT / "ea_deep.h5", "w", libver="latest") as f:
        f.create_dataset("data", data=deep, maxshape=(None, 2), chunks=(1, 2))
    oracle = {}
    for name in ("ea_deep.h5", "ea_latest.h5"):
        with h5py.File(OUT / name, "r") as f:
            values = f["data"][()]
        rows, cols = values.shape
        expected = (np.arange(rows * cols, dtype="<f4").reshape(rows, cols) + 1)
        assert np.array_equal(values, expected), name
        oracle[name] = hashlib.sha256(values.astype("<f4").tobytes()).hexdigest()
    manifest = {
        "generator": "make_fixtures.py",
        "h5py_values_sha256": oracle,
        "h5py": h5py.__version__,
        "hdf5": h5py.version.hdf5_version,
        "files": {
            p.name: hashlib.sha256(p.read_bytes()).hexdigest()
            for p in sorted(OUT.glob("*.h5"))
        },
    }
    (OUT / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
