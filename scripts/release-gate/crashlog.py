"""pytest plugin: per-worker faulthandler log plus the running test id (survives a native crash).
Directory from env CRASHLOG_DIR."""
import faulthandler, os
_f = None
def pytest_configure(config):
    global _f
    d = os.environ.get("CRASHLOG_DIR")
    if not d: return
    os.makedirs(d, exist_ok=True)
    _f = open(os.path.join(d, f"fault-{os.environ.get('PYTEST_XDIST_WORKER', 'main')}-{os.getpid()}.log"), "a", buffering=1)
    faulthandler.enable(_f, all_threads=True)
def pytest_runtest_logstart(nodeid, location):
    if _f: _f.write(f"START {nodeid}\n"); _f.flush(); os.fsync(_f.fileno())
def pytest_runtest_logfinish(nodeid, location):
    if _f: _f.write(f"END {nodeid}\n"); _f.flush()
