"""Generate the synthetic steady XS identity-fallback fixtures and their SHA256 manifest.

Usage: python -I make_fixtures.py   (writes next to this file; requires h5py + numpy only)

Python reference: ras-commander ``HdfResultsPlan._get_steady_cross_section_attributes``
(branch integration/native-release-20261022) reads the steady identity from
``Results/Steady/Output/Geometry Info/Cross Section Attributes`` and, when that is
absent, from ``Geometry/Cross Sections/Attributes`` (station field ``RS``), aligning
result columns by order and count. Each file isolates one case of that rule.
"""
import hashlib
import json
from pathlib import Path

import h5py
import numpy as np

HERE = Path(__file__).resolve().parent
GEOM = "Geometry/Cross Sections/Attributes"
RESULT_IDENTITY = "Results/Steady/Output/Geometry Info/Cross Section Attributes"
PACKED_IDENTITY = "Results/Steady/Output/Geometry Info/Cross Section Only"
STEADY = "Results/Steady/Output/Output Blocks/Base Output/Steady Profiles"
UNSTEADY = "Results/Unsteady/Output/Output Blocks/Base Output/Unsteady Time Series"
# HEC-RAS 6.x geometry table layout (fixed ASCII text, RS station field).
GEOM_DT = np.dtype([("River", "S16"), ("Reach", "S16"), ("RS", "S8"), ("Name", "S16"),
                    ("Len Channel", "<f4")])
ROWS = [(b"River A", b"Reach A", b"1000", b"", 125.0), (b"River A", b"Reach A", b"900", b"", 1.0e30)]
RESULT_DT = np.dtype([("River", "S16"), ("Reach", "S16"), ("Station", "S8"), ("Name", "S16")])


def geometry(f, rows=ROWS, dtype=GEOM_DT):
    f.create_dataset(GEOM, data=np.array(rows, dtype=dtype))


def steady_results(f, sections=2):
    f[STEADY + "/Profile Names"] = np.array([b"10-percent AEP", b"1-percent AEP"], dtype="S16")
    f[STEADY + "/Cross Sections/Water Surface"] = np.arange(2 * sections, dtype="<f4").reshape(2, sections) + 100
    f[STEADY + "/Cross Sections/Flow"] = np.arange(2 * sections, dtype="<f8").reshape(2, sections) + 1000


def result_identity(f, stations=(b"1000", b"900"), dtype=RESULT_DT):
    rows = [(b"River A", b"Reach A", s, b"") for s in stations]
    f.create_dataset(RESULT_IDENTITY, data=np.array(rows, dtype=dtype))


def case(name):
    def register(fn):
        CASES[name] = fn
        return fn
    return register


CASES = {}


@case("results_present.h5")
def _(f):
    geometry(f)
    result_identity(f)
    steady_results(f)


@case("results_present_mismatch.h5")
def _(f):
    # Present results-side table that contradicts geometry order stays Corrupt.
    geometry(f)
    result_identity(f, stations=(b"900", b"1000"))
    steady_results(f)


@case("results_present_rs_field.h5")
def _(f):
    # Results-side table with RS instead of Station: unchanged engine refusal (Python would
    # accept it through its case-insensitive station/rs lookup).
    geometry(f)
    dt = np.dtype([("River", "S16"), ("Reach", "S16"), ("RS", "S8"), ("Name", "S16")])
    result_identity(f, dtype=dt)
    steady_results(f)


@case("packed_only.h5")
def _(f):
    # Packed results-side identity (16/16/8 plus separators) keeps precedence over geometry.
    geometry(f)
    packed = [b"River A".ljust(16) + b" " + b"Reach A".ljust(16) + b" " + s.ljust(8)
              for s in (b"1000", b"900")]
    f[PACKED_IDENTITY] = np.array(packed, dtype="S42")
    steady_results(f)


@case("geometry_fallback.h5")
def _(f):
    # HEC-RAS 6.x layout: no results-side identity table; geometry keeps the identity (RS).
    geometry(f)
    steady_results(f)


@case("geometry_fallback_unsteady.h5")
def _(f):
    # Python's unsteady readers require the results-side table: no fallback is mirrored.
    geometry(f)
    f[UNSTEADY + "/Time"] = np.array([0.0, 0.5])
    f[UNSTEADY + "/Cross Sections/Water Surface"] = np.zeros((2, 2), dtype="<f4")


@case("both_absent.h5")
def _(f):
    steady_results(f)


@case("count_mismatch.h5")
def _(f):
    # Three geometry sections, two result columns: the order/count alignment fails.
    geometry(f, rows=ROWS + [(b"River A", b"Reach A", b"800", b"", 10.0)])
    steady_results(f, sections=2)


@case("unproven_river.h5")
def _(f):
    # Variable-length River: not proven fixed text.
    dt = np.dtype([("River", h5py.string_dtype()), ("Reach", "S16"), ("RS", "S8")])
    geometry(f, rows=[(r[0].decode(), r[1], r[2]) for r in ROWS], dtype=dt)
    steady_results(f)


@case("unproven_station.h5")
def _(f):
    # Python prefers a `Station` field over `RS`; here it is numeric, not proven text.
    dt = np.dtype([("River", "S16"), ("Reach", "S16"), ("RS", "S8"), ("Station", "<f4")])
    geometry(f, rows=[(r[0], r[1], r[2], 1.0) for r in ROWS], dtype=dt)
    steady_results(f)


@case("station_and_rs_disagree.h5")
def _(f):
    # Python would align results by `Station`; it contradicts the RS geometry identity.
    dt = np.dtype([("River", "S16"), ("Reach", "S16"), ("RS", "S8"), ("Station", "S8")])
    geometry(f, rows=[(b"River A", b"Reach A", b"1000", b"900"),
                      (b"River A", b"Reach A", b"900", b"1000")], dtype=dt)
    steady_results(f)


@case("case_ambiguous_rs.h5")
def _(f):
    # `RS` and `rs` both present: Python's lower-cased lookup would silently keep the last.
    dt = np.dtype([("River", "S16"), ("Reach", "S16"), ("RS", "S8"), ("rs", "S8")])
    geometry(f, rows=[(r[0], r[1], r[2], r[2]) for r in ROWS], dtype=dt)
    steady_results(f)


@case("lowercase_fields.h5")
def _(f):
    # Python accepts river/reach/rs by lower-cased lookup; the engine geometry table
    # (`cross_sections`) requires the canonical River/Reach/RS names.
    dt = np.dtype([("river", "S16"), ("reach", "S16"), ("rs", "S8")])
    geometry(f, rows=[r[:3] for r in ROWS], dtype=dt)
    steady_results(f)


def main():
    manifest = {}
    for name, build in CASES.items():
        path = HERE / name
        with h5py.File(path, "w", libver="earliest", track_order=False) as f:
            build(f)
        manifest[name] = hashlib.sha256(path.read_bytes()).hexdigest()
    (HERE / "manifest.json").write_text(json.dumps({"fixtures": manifest}, indent=2) + "\n")


if __name__ == "__main__":
    main()
