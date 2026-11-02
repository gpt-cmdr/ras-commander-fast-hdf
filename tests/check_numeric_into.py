"""Installed-wheel checks for exact caller-owned numeric output buffers."""

from pathlib import Path
import unittest
import h5py
import numpy as np
import ras_commander_fast_hdf as native

FIXTURE = Path(__file__).resolve().parent / "fixtures/hdf_numeric/numeric_types_v1.h5"


def raw(a):
    return memoryview(np.ascontiguousarray(a).reshape(-1).view(np.uint8))


class NumericIntoTests(unittest.TestCase):
    def test_exact_full_reads_and_stable_allocation(self):
        with h5py.File(FIXTURE) as h, native.HdfSession(str(FIXTURE)) as session:
            paths = []
            h.visititems(
                lambda name, obj: (
                    paths.append(name)
                    if isinstance(obj, h5py.Dataset)
                    and name.startswith(
                        ("int/", "float/", "shape/", "fill/", "filter/", "endian/")
                    )
                    else None
                )
            )
            for path in paths:
                with self.subTest(path=path):
                    expected = np.asarray(
                        h[path][()], dtype=h[path].dtype.newbyteorder("=")
                    )
                    out = np.full(expected.shape, 17, dtype=expected.dtype)
                    if not h[path].dtype.isnative:
                        before = out.copy()
                        with self.assertRaises(ValueError):
                            session.read_numeric_into(path, out)
                        self.assertEqual(raw(out), raw(before))
                        continue
                    pointer = out.__array_interface__["data"][0]
                    for _ in range(3):
                        self.assertIsNone(session.read_numeric_into(path, out))
                        self.assertEqual(pointer, out.__array_interface__["data"][0])
                        self.assertEqual(raw(out), raw(expected))
                    if out.size:
                        out.flat[0] = 71
                        session.read_numeric_into(path, out)
                        self.assertEqual(raw(out), raw(expected))
                        independent = session.read_numeric(path)
                        independent.flat[0] = 72
                        session.read_numeric_into(path, out)
                        self.assertEqual(raw(out), raw(expected))

    def test_preflight_rejection_never_writes(self):
        with native.HdfSession(str(FIXTURE)) as session:
            for path, out, options in [
                ("shape/chunked_2d_f4", np.full((37, 23), 17, dtype="f8"), {}),
                ("shape/chunked_2d_f4", np.full((23, 37), 17, dtype="f4"), {}),
                (
                    "shape/chunked_2d_f4",
                    np.asfortranarray(np.full((37, 23), 17, dtype="f4")),
                    {},
                ),
                ("shape/chunked_2d_f4", np.full((37, 46), 17, dtype="f4")[:, ::2], {}),
                (
                    "shape/chunked_2d_f4",
                    np.full((37, 23), 17, dtype="f4"),
                    {"max_bytes": 1},
                ),
                ("missing", np.full((37, 23), 17, dtype="f4"), {}),
                ("encoding/i32_precision24_offset4", np.full((4,), 17, dtype="i4"), {}),
                ("class/bool", np.array([True, False]), {}),
                ("int/i4_bounds", np.full(3, 17, dtype=">i4"), {}),
            ]:
                with self.subTest(path=path, dtype=out.dtype, options=options):
                    before = out.copy()
                    with self.assertRaises((ValueError, TypeError, KeyError)):
                        session.read_numeric_into(path, out, **options)
                    self.assertEqual(raw(out), raw(before))
            readonly = np.full(3, 17, dtype="i4")
            readonly.flags.writeable = False
            with self.assertRaises(ValueError):
                session.read_numeric_into("int/i4_bounds", readonly)
            unaligned = np.ndarray((3,), dtype="i4", buffer=bytearray(13), offset=1)
            with self.assertRaises(ValueError):
                session.read_numeric_into("int/i4_bounds", unaligned)

    def test_policy_refusal_and_late_checksum_failure_recover(self):
        import tempfile

        policy = FIXTURE.parent.parent / "reader_policy/filter_lz4_then_deflate.h5"
        out = np.full((6, 8), 17, dtype="f8")
        before = out.copy()
        with native.HdfSession(str(policy)) as session:
            with self.assertRaises(native.UnsupportedError):
                session.read_numeric_into("data", out)
        self.assertEqual(raw(out), raw(before))

        with h5py.File(FIXTURE) as h:
            ds = h["filter/fletcher32_i8"]
            offset = ds.id.get_chunk_info(ds.id.get_num_chunks() - 1).byte_offset
            expected = h["int/i64_bounds"][()]
        data = bytearray(FIXTURE.read_bytes())
        data[offset] ^= 1
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "checksum-failure.h5"
            path.write_bytes(data)
            with native.HdfSession(str(path)) as session:
                out = np.full(300, 17, dtype="i8")
                pointer = out.__array_interface__["data"][0]
                with self.assertRaisesRegex(native.CorruptDataError, "checksum"):
                    session.read_numeric_into("filter/fletcher32_i8", out)
                np.testing.assert_array_equal(out[:64], np.arange(64))
                session.read_numeric_into("int/i64_bounds", out[:6])
                self.assertEqual(pointer, out.__array_interface__["data"][0])
                self.assertEqual(raw(out[:6]), raw(expected))

    def test_closed_session_and_null_refuse(self):
        session = native.HdfSession(str(FIXTURE))
        session.close()
        out = np.full(3, 17, dtype="i4")
        with self.assertRaisesRegex(ValueError, "closed"):
            session.read_numeric_into("int/i4_bounds", out)
        with native.HdfSession(
            str(FIXTURE.parent.parent / "dataspace/dataspace.h5")
        ) as session:
            out = np.array(17, dtype="f4")
            with self.assertRaises(ValueError):
                session.read_numeric_into("datasets/null_f4", out)
            self.assertEqual(out.item(), 17)


if __name__ == "__main__":
    unittest.main()
