#!/bin/bash
# Usage: build_wheel.sh <tag> [engine-checkout] [features]
# Builds the engine wheel from a checkout (default $ENG) and installs it into the gate venv.
# Run it through with_gate_queue.sh: release builds are heavy.
. "${GATE_ENV:-$(dirname "$0")/env.sh}"
T=${1:-dev}; SRC=${2:-$ENG}; FEAT=${3:-exports-all,exports-pmtiles,extension-module}; OUT=$W/wheels/$T; rm -rf $OUT; mkdir -p $OUT
(cd $SRC/python && $PY -m maturin build --release --locked --interpreter $PY --features $FEAT --out $OUT) > $RUN/logs/wheel-$T.log 2>&1 || { echo build failed; tail -40 $RUN/logs/wheel-$T.log; exit 1; }
uv pip install --python $PY --reinstall --no-deps $OUT/*.whl >> $RUN/logs/wheel-$T.log 2>&1 || exit 1
$PY -c "import ras_commander_fast_hdf as n; print(n.__file__, n.API_VERSION, n.ENGINE_COMMIT)"
ls -la $OUT
