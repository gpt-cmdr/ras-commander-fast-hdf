# Release gate scripts

The release gate: the ras-commander equivalence suites and the full
ras-commander suite in h5py, auto and required mode, run against one engine
wheel. Compare two runs case by case (junit XML in `$RUN/logs/<tag>/`).

The scripts run in Git Bash on Windows and in bash on Linux. `env.sh` detects
which from `uname -s` and sets the venv's Python (`Scripts/python.exe` or
`bin/python`) and the separator of the path lists `PYTHONPATH` and
`RAS_FLUVIAL_FIXTURE_DIRS` (`;` on Windows, `:` elsewhere).
`requirements-release.txt` installs `pywin32` only on Windows. Only Windows
prepares the HEC-RAS GDAL bridge and removes conda from `PATH`. The `D:/`
defaults in `env.sh` are the Windows gate machine's; on Linux set `W`, `FX`
and the other paths. `with_gate_queue.sh` keeps its lock in `/tmp/ras-perf`
on Linux unless `GATE_LOCK_DIR` is set.

Linux was qualified for 0.5.1 this way on CLB's Slurm cluster. One job ran
`setup_env.sh` with the released manylinux wheel, then
`full_run.sh <tag> route2 route3 nonhdf fluvial` with
`ROUTE2_MANIFEST` pointing at the full route-2 manifest. It ran inside the CI
runner's Apptainer image, with all state in the job's `/scratch/<jobid>`.

| File | Purpose |
|---|---|
| `env.sh` | Paths (work dir, checkouts, venv, fixtures). Copy and edit, or preset the variables; `GATE_ENV` names another file. |
| `setup_env.sh [engine]` | Creates the venv from `requirements-release.txt`, installs the engine (default: the released wheel from PyPI) and the ras-commander checkout, and prepares the HEC-RAS GDAL bridge once. |
| `build_wheel.sh <tag> [engine-checkout]` | Builds and installs an engine wheel from source. |
| `full_run.sh <tag> [stage...]` | Runs the stages; logs and junit XML go to `$RUN/logs/<tag>/`. |
| `crashlog.py` | pytest plugin (on `PYTHONPATH` via `$RUN/scripts`): per-worker faulthandler log with the running test id. |
| `with_gate_queue.sh "<desc>" <cmd...>` | First-in, first-out queue for the shared machine lock (`GATE_RUNNING`); compatible with the older `with_gate_lock.sh`. |

Stages: `existing quarantine route1 route2 route3 extents nonhdf fluvial
main-python main-auto main-required`. The `main-*` stages run `tests/` without
the `tests/rust_*` folders (those are the earlier stages).

```bash
export W=D:/CLB-Dev/my-gate CLONE=D:/CLB-Dev/my-gate/ras-commander RUN=D:/CLB-Dev/my-gate/run
# Linux: export W=/data/my-gate CLONE=/data/my-gate/ras-commander RUN=/data/my-gate/run FX=/data/fixtures
mkdir -p $RUN/scripts && cp crashlog.py $RUN/scripts/
bash setup_env.sh                       # or: bash setup_env.sh path/to/wheel.whl
bash with_gate_queue.sh "my gate" bash full_run.sh mytag main-python main-auto main-required
```

Run every full stage through `with_gate_queue.sh` on a shared machine.
