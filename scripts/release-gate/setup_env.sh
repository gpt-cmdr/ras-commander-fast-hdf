#!/bin/bash
# Usage: setup_env.sh [engine-requirement-or-wheel]   (default: the released engine on PyPI)
# Builds the gate venv from the pinned requirements, installs the engine and the
# ras-commander checkout, and prepares the HEC-RAS GDAL bridge once.
. "${GATE_ENV:-$(dirname "$0")/env.sh}"
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
ENGINE=${1:-ras-commander-fast-hdf}
[ -x $PY ] || uv venv --python 3.13 $W/venv
uv pip install --python $PY -r "$HERE/requirements-release.txt"
uv pip install --python $PY --no-deps "$ENGINE"
uv pip install --python $PY --no-deps -e "$CLONE"
# RasMapperLib needs a GDAL junction beside python.exe. The library creates it on first
# load with a 15 s PowerShell call, which timed out under gate load and failed every
# RasMapperLib test in that worker; create it once here instead (Windows with HEC-RAS only).
$PY - <<'EOF' || echo "HEC-RAS GDAL bridge not prepared (no HEC-RAS install?); RasMapperLib tests will skip or fail"
import sys
from ras_commander._gdal_runtime import ensure_python_gdal_junction
from ras_commander.dotnet.clr_bootstrap import find_hecras_install
sys.exit(0 if ensure_python_gdal_junction(find_hecras_install()) else 1)
EOF
$PY -c "import ras_commander, ras_commander_fast_hdf as n, sys; print(ras_commander.__file__, n.__file__, n.API_VERSION, n.ENGINE_COMMIT, sys.version)"
