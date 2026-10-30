# Release gate scripts

The Windows release gate: the ras-commander equivalence suites and the full
ras-commander suite in h5py, auto and required mode, run against one engine
wheel. Compare two runs case by case (junit XML in `$RUN/logs/<tag>/`).

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
mkdir -p $RUN/scripts && cp crashlog.py $RUN/scripts/
bash setup_env.sh                       # or: bash setup_env.sh path/to/wheel.whl
bash with_gate_queue.sh "my gate" bash full_run.sh mytag main-python main-auto main-required
```

Run every full stage through `with_gate_queue.sh` on a shared machine.
