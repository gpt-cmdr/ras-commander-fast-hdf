//! Panic containment at the Python boundary.
//!
//! Every `#[pyfunction]` and `#[pymethods]` body runs inside [`guard`]. A Rust
//! panic (a bug, never a property of the input) becomes an ordinary
//! `Exception` instead of PyO3's `PanicException`, which derives from
//! `BaseException` and so escapes the `except Exception` h5py fallback.
//!
//! When `ras_commander.fast_hdf` is loaded, the error is its
//! `FastHdfPanicError`, a `FastHdfError` subclass, so required-native mode
//! reports it like any other native refusal. Standalone use of the extension
//! gets [`NativePanicError`], a `RuntimeError` subclass.
//!
//! Aborts (stack overflow, allocation failure) cannot be caught here; the
//! engine prevents those on file input by bounding resolution depth and
//! allocations before they happen.
//!
//! [`install_quiet_hook`] (called when the module is imported) silences Rust's
//! default panic message on stderr: the Python exception already carries the
//! message. Setting `RUST_BACKTRACE` (to anything but `0`) restores the default
//! output, with a backtrace, for debugging.
//!
//! The panic-injection hooks (`_panic_for_test` here and on `HdfSession`) are
//! compiled only with the `test-hooks` feature; release wheels leave it off.
use std::any::Any;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Once;

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;

pyo3::create_exception!(
    ras_commander_fast_hdf,
    NativePanicError,
    PyRuntimeError,
    "A Rust panic inside the native extension, contained at the Python boundary."
);

/// Run `f`, converting a panic into a Python exception.
pub(crate) fn guard<T>(f: impl FnOnce() -> PyResult<T>) -> PyResult<T> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(result) => result,
        Err(payload) => Err(panic_error(payload.as_ref())),
    }
}

/// Run `f` without the GIL, returning a panic as its message. Use it where a
/// lock is held, so the panic is caught before the lock guard unwinds (no
/// poisoning); build the Python error later with [`panic_error_from`].
pub(crate) fn contain<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    catch_unwind(AssertUnwindSafe(f)).map_err(|payload| panic_message(payload.as_ref()))
}

/// The Python error for a panic message captured by [`contain`].
pub(crate) fn panic_error_from(message: String) -> PyErr {
    Python::attach(|py| python_panic_error(py, &message))
        .unwrap_or_else(|| NativePanicError::new_err(message))
}

pub(crate) fn panic_message(payload: &(dyn Any + Send)) -> String {
    let detail = payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "non-string panic payload".to_owned());
    format!("native extension panicked: {detail}")
}

fn panic_error(payload: &(dyn Any + Send)) -> PyErr {
    panic_error_from(panic_message(payload))
}

/// `ras_commander.fast_hdf.FastHdfPanicError(message)` when that module is
/// already imported. It is never imported from here.
fn python_panic_error(py: Python<'_>, message: &str) -> Option<PyErr> {
    let modules = py.import("sys").ok()?.getattr("modules").ok()?;
    let module = modules
        .call_method1("get", ("ras_commander.fast_hdf",))
        .ok()?;
    if module.is_none() {
        return None;
    }
    let class = module.getattr("FastHdfPanicError").ok()?;
    let instance = class.call1((message,)).ok()?;
    instance
        .is_instance_of::<pyo3::exceptions::PyException>()
        .then(|| PyErr::from_value(instance))
}

/// Whether a panic should print Rust's default message: only when the user
/// asked for backtraces (`RUST_BACKTRACE` set and not `0`).
fn panic_output_requested(rust_backtrace: Option<&std::ffi::OsStr>) -> bool {
    rust_backtrace.is_some_and(|value| value != "0")
}

/// Replace the panic hook of this extension (its own copy of the Rust standard
/// library; other extensions and Rust programs are not affected) with one that
/// prints nothing, unless `RUST_BACKTRACE` asks for output. Every panic in the
/// extension is contained by [`guard`] or [`contain`] (rayon re-raises worker
/// panics in the calling thread), and the message travels in the Python
/// exception. Idempotent.
pub(crate) fn install_quiet_hook() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let default = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if panic_output_requested(std::env::var_os("RUST_BACKTRACE").as_deref()) {
                default(info);
            }
        }));
    });
}

/// Test hook (feature `test-hooks`): panic inside the guard, as an engine bug would.
#[cfg(feature = "test-hooks")]
#[pyfunction]
pub(crate) fn _panic_for_test(message: &str) -> PyResult<()> {
    guard(|| panic!("{message}"))
}

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("NativePanicError", m.py().get_type::<NativePanicError>())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::process::Command;

    const CHILD: &str = "python::panic_guard::tests::child_panics_under_the_quiet_hook";

    #[test]
    fn output_only_when_backtraces_are_requested() {
        assert!(!panic_output_requested(None));
        assert!(!panic_output_requested(Some(OsStr::new("0"))));
        assert!(panic_output_requested(Some(OsStr::new("1"))));
        assert!(panic_output_requested(Some(OsStr::new("full"))));
    }

    /// Run [`child_panics_under_the_quiet_hook`] in a fresh test process, with
    /// output capture off, and return its stderr.
    fn child_stderr(rust_backtrace: Option<&str>) -> String {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args([
            CHILD,
            "--exact",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ]);
        match rust_backtrace {
            Some(value) => command.env("RUST_BACKTRACE", value),
            None => command.env_remove("RUST_BACKTRACE"),
        };
        let output = command.output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        assert!(
            output.status.success(),
            "{stdout}
{stderr}"
        );
        assert!(stdout.contains("1 passed"), "{stdout}");
        stderr
    }

    #[test]
    fn contained_panics_print_nothing() {
        for rust_backtrace in [None, Some("0")] {
            let stderr = child_stderr(rust_backtrace);
            assert!(!stderr.contains("quiet hook test panic"), "{stderr}");
            assert!(!stderr.contains("panicked at"), "{stderr}");
        }
    }

    #[test]
    fn rust_backtrace_restores_the_default_message() {
        let stderr = child_stderr(Some("1"));
        assert!(stderr.contains("quiet hook test panic"), "{stderr}");
    }

    #[test]
    #[ignore = "child process of contained_panics_print_nothing"]
    fn child_panics_under_the_quiet_hook() {
        install_quiet_hook();
        install_quiet_hook();
        let message = contain(|| -> () { panic!("quiet hook test panic") }).unwrap_err();
        assert_eq!(message, "native extension panicked: quiet hook test panic");
        // A worker-thread panic re-raised in the caller, as rayon does.
        let message = contain(|| {
            std::thread::spawn(|| panic!("quiet hook test panic in a worker"))
                .join()
                .map_err(std::panic::resume_unwind)
                .unwrap()
        })
        .unwrap_err();
        assert_eq!(
            message,
            "native extension panicked: quiet hook test panic in a worker"
        );
    }

    #[test]
    fn a_guarded_panic_is_a_python_exception_with_its_message() {
        pyo3::Python::initialize();
        Python::attach(|py| {
            let error = guard(|| -> PyResult<()> { panic!("guarded test panic") }).unwrap_err();
            assert!(error.is_instance_of::<NativePanicError>(py));
            assert!(error.is_instance_of::<pyo3::exceptions::PyException>(py));
            assert_eq!(
                error.value(py).to_string(),
                "native extension panicked: guarded test panic"
            );
        });
    }
}
