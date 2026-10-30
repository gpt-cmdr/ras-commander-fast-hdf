#!/bin/bash
# Usage: full_run.sh <tag> [stage...]
# Stages: existing quarantine route1 route2 route3 extents nonhdf fluvial main-python main-auto main-required
# Settings come from env.sh next to this script, or from the file named by GATE_ENV.
. "${GATE_ENV:-$(dirname "$0")/env.sh}"
T=${1:-run}; shift; STAGES=${*:-existing quarantine route1 route2 route3 extents nonhdf fluvial main-python main-auto main-required}
L=$RUN/logs/$T; mkdir -p $L $W/cwd; cd $W/cwd || exit 1
M=$RUN/manifests
export RAS_ROUTE1_FIXTURES=$M/fixtures-route1.json
export PYTHONPATH="$CLONE;$CLONE/tests;$CLONE/tests/rust_engine_bridge;$RUN/scripts" CRASHLOG_DIR=$L/fault
unset RAS_COMMANDER_FAST_HDF
P="$PY -m pytest --import-mode=importlib -q -p no:cacheprovider -p no:faulthandler -p crashlog -rs"
# The routed-equivalence folders (tests/rust_*) run in their own stages above, so the
# main suites leave them out. One --ignore per folder: --ignore-glob="*/rust_*" is
# matched against the native path, so on Windows (backslashes) it excluded nothing.
RUST_IGNORES=$(for d in "$CLONE"/tests/rust_*/; do printf ' --ignore=%s' "${d%/}"; done)
{ $PY -c 'import ras_commander,ras_commander_fast_hdf as n,pandas,numpy,h5py; print(ras_commander.__file__, n.__file__, n.API_VERSION, n.ENGINE_COMMIT, "pandas", pandas.__version__, "numpy", numpy.__version__, "h5py", h5py.__version__)'
  git -c safe.directory='*' -C $CLONE rev-parse HEAD; git -c safe.directory='*' -C $CLONE status --short; } > $L/env.txt 2>&1
merge() { cat $1.gw* > $1 2>/dev/null; rm -f $1.gw*; }
for st in $STAGES; do
  date +"%F %T start $st" >> $L/done.txt
  case $st in
  existing) $P -s "$CLONE/tests/rust_consumer_contract" "$CLONE/tests/rust_engine_bridge" "$CLONE/tests/rust_python_audit_compat" "$CLONE/tests/rust_fim_wave1" --basetemp=$W/tmp-$T-a --junitxml=$L/existing.xml > $L/existing.log 2>&1 ;;
  quarantine) $P -n 4 "$CLONE/tests/rust_mesh_quarantine" --basetemp=$W/tmp-$T-q --junitxml=$L/quarantine.xml > $L/quarantine.log 2>&1 ;;
  route1) RAS_ROUTE1_RESULTS=$L/route1.jsonl $P -n 6 "$CLONE/tests/rust_route_1" --basetemp=$W/tmp-$T-b --junitxml=$L/route1.xml > $L/route1.log 2>&1; rc=$?; merge $L/route1.jsonl; (exit $rc) ;;
  route2) RAS_ROUTE2_RESULTS=$L/route2.jsonl RAS_ROUTE2_FIXTURES=${ROUTE2_MANIFEST:-$M/fixtures-route2.json} $P -n 6 "$CLONE/tests/rust_route_2" --basetemp=$W/tmp-$T-c --junitxml=$L/route2.xml > $L/route2.log 2>&1; rc=$?; merge $L/route2.jsonl; (exit $rc) ;;
  route3) RAS_ROUTE2_FIXTURES=${ROUTE2_MANIFEST:-$M/fixtures-route2.json} RAS_ROUTE3_LEGACY_EXCERPTS=$ENG/tests/fixtures/legacy_location RAS_ROUTE3_RESULTS=$L/route3.jsonl $P "$CLONE/tests/rust_route_3" --basetemp=$W/tmp-$T-d --junitxml=$L/route3.xml > $L/route3.log 2>&1 ;;
  extents) RAS_ROUTE_EXTENTS_RESULTS=$L/extents.jsonl RAS_ROUTE_EXTENTS_FIXTURES=$RAS_ROUTE1_FIXTURES $P -n 6 "$CLONE/tests/rust_route_extents" --basetemp=$W/tmp-$T-e --junitxml=$L/extents.xml > $L/extents.log 2>&1; rc=$?; merge $L/extents.jsonl; (exit $rc) ;;
  nonhdf) RAS_ROUTE2_FIXTURES=${ROUTE2_MANIFEST:-$M/fixtures-route2.json} RAS_ROUTE_NONHDF_RESULTS=$L/nonhdf.jsonl $P -n 6 "$CLONE/tests/rust_route_nonhdf" --basetemp=$W/tmp-$T-f --junitxml=$L/nonhdf.xml > $L/nonhdf.log 2>&1; rc=$?; merge $L/nonhdf.jsonl; (exit $rc) ;;
  fluvial) RAS_FLUVIAL_PUBLIC_FIXTURES=$FX/structures-fixtures RAS_MESH_DIAGNOSTIC_FIXTURES=$FX/structures-fixtures RAS_FLUVIAL_FIXTURE_DIRS="$FX/structures-fixtures;$FX/wave2-fixtures;$FX/wave3-fixtures;$FX/golden" $P -n 4 "$CLONE/tests/rust_fluvial_public" "$CLONE/tests/test_rust_fluvial_contract.py" "$CLONE/tests/test_mesh_polygon_diagnostics_parity.py" "$CLONE/tests/test_mesh_polygon_ring_contract.py" --basetemp=$W/tmp-$T-g --junitxml=$L/fluvial.xml > $L/fluvial.log 2>&1 ;;
  main-python) RAS_COMMANDER_FAST_HDF=0 $P -n 8 "$CLONE/tests" $RUST_IGNORES --timeout=900 --basetemp=$W/tmp-$T-h --junitxml=$L/main-python.xml > $L/main-python.log 2>&1 ;;
  main-auto) RAS_COMMANDER_FAST_HDF=auto $P -n 8 "$CLONE/tests" $RUST_IGNORES --timeout=900 --basetemp=$W/tmp-$T-i --junitxml=$L/main-auto.xml > $L/main-auto.log 2>&1 ;;
  main-required) RAS_COMMANDER_FAST_HDF=required $P -n 8 "$CLONE/tests" $RUST_IGNORES --timeout=900 --basetemp=$W/tmp-$T-j --junitxml=$L/main-required.xml > $L/main-required.log 2>&1 ;;
  esac
  echo "$st $? $(tail -1 $L/$st.log 2>/dev/null)" >> $L/done.txt
done
echo FINISHED >> $L/done.txt
