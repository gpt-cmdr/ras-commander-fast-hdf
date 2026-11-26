# Release-gate settings. Copy this file next to your copy of the scripts (or point
# GATE_ENV at it) and set the paths; every value can also be preset in the environment.
# Windows (Git Bash/MSYS) or Linux/macOS: the venv layout and the separator of path lists
# (PYTHONPATH, RAS_FLUVIAL_FIXTURE_DIRS) differ. The D:/ defaults are the Windows gate's;
# on Linux/macOS set W, FX and the other paths.
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) GATE_WINDOWS=1; GATE_PATHSEP=';'; GATE_VENV_PY=Scripts/python.exe ;;
  *) GATE_WINDOWS=0; GATE_PATHSEP=':'; GATE_VENV_PY=bin/python ;;
esac
export GATE_WINDOWS GATE_PATHSEP
export W=${W:-D:/CLB-Dev/release-gate}                    # work dir: venv, cwd, tmp-*, wheels
export CLONE=${CLONE:-$W/ras-commander}                   # ras-commander checkout under test
export ENG=${ENG:-$W/ras-commander-fast-hdf}              # engine checkout (route3 legacy excerpts)
export RUN=${RUN:-$W/run}                                 # logs/ and manifests/; scripts on PYTHONPATH (crashlog)
export FX=${FX:-D:/CLB-Dev/ras-native-release/fixtures}   # fluvial/structure fixture corpus
export PY=${PY:-$W/venv/$GATE_VENV_PY}
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$W/target}
export CARGO_NET_GIT_FETCH_WITH_CLI=true
# A conda install on PATH ahead of Git (common in interactive profiles) mixes two MSYS
# runtimes: Git's bash with conda's cygpath (/cygdrive/... paths Git's bash cannot open),
# and conda's GDAL/PROJ/HDF5 DLLs beside the gate venv. Keep conda off the gate's PATH.
# (Windows only: Linux and macOS do not load shared libraries from PATH.)
if [ "$GATE_WINDOWS" = 1 ]; then
  export PATH=$(printf '%s' "$PATH" | tr ':' '\n' | grep -v -i -E '/(ana|mini)conda[^/]*(/|$)' | paste -sd:)
fi
mkdir -p $RUN/logs $W/wheels
