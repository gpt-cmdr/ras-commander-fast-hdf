//! A cap on the memory held by dataset reads in progress, across worker
//! threads and concurrent calls.
//!
//! Every dataset read (`src/hdf/reader.rs`) reserves, before the reader
//! allocates anything, an estimate of the memory it will hold while it runs:
//! the decoded values it returns, plus up to two chunk buffers (stored and
//! decoded) for each worker that decodes chunks in parallel. The reservation is
//! released when the read returns. Values handed back to the caller are not
//! counted after that.
//!
//! - A read whose estimate alone exceeds the budget is refused at once with
//!   `Unsupported{ResourceLimit}`.
//! - Otherwise the read waits until enough of the budget is free, for at most
//!   the budget's wait time, and is then refused the same way.
//! - A read started on a rayon worker thread (a session's parallel reads run
//!   inside its pool) does not wait: it is admitted if the budget has room and
//!   refused otherwise. A blocked worker could deadlock the pool: while it waits
//!   in a parallel join, rayon may run another read on it, and the read holding
//!   the budget would then sit below the blocked one on the same stack.
//!
//! The default budget ([`DEFAULT_DECODE_BUDGET_BYTES`], 4 GiB) is twice the
//! per-call default ([`super::DEFAULT_READ_MAX_BYTES`]), so a read never waits
//! unless other reads already hold gigabytes. Sessions share one process-wide
//! budget unless given their own ([`super::ReadOnlyHdfSession::set_decode_budget`]);
//! `RAS_FAST_HDF_DECODE_BUDGET_BYTES` and `RAS_FAST_HDF_DECODE_BUDGET_WAIT_MS`
//! set the process-wide one when it is first used.
use crate::{Error, ErrorKind, Result};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

/// Default process-wide in-flight decode budget: 4 GiB.
pub const DEFAULT_DECODE_BUDGET_BYTES: u64 = 4 * 1024 * 1024 * 1024;
/// Default longest wait for budget before a read is refused: 30 seconds.
pub const DEFAULT_DECODE_BUDGET_WAIT: Duration = Duration::from_secs(30);
/// Environment variable that sets the process-wide budget in bytes.
pub const DECODE_BUDGET_BYTES_ENV: &str = "RAS_FAST_HDF_DECODE_BUDGET_BYTES";
/// Environment variable that sets the process-wide wait in milliseconds.
pub const DECODE_BUDGET_WAIT_MS_ENV: &str = "RAS_FAST_HDF_DECODE_BUDGET_WAIT_MS";

/// A byte budget shared by the reads of every session that holds it.
#[derive(Debug)]
pub struct DecodeBudget {
    max_bytes: u64,
    wait: Duration,
    state: Mutex<State>,
    released: Condvar,
}

#[derive(Debug, Default)]
struct State {
    in_flight: u64,
    peak: u64,
    waits: u64,
    refusals: u64,
}

/// A snapshot of a [`DecodeBudget`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct DecodeBudgetStats {
    /// The budget, in bytes.
    pub max_bytes: u64,
    /// Longest wait for budget before a read is refused.
    pub wait: Duration,
    /// Bytes reserved by reads in progress.
    pub in_flight_bytes: u64,
    /// Largest `in_flight_bytes` seen.
    pub peak_in_flight_bytes: u64,
    /// Reads that had to wait for budget.
    pub waits: u64,
    /// Reads refused for budget (too large, or the wait ran out).
    pub refusals: u64,
}

impl DecodeBudget {
    /// A budget of `max_bytes` (at least 1) with the given longest wait.
    pub fn new(max_bytes: u64, wait: Duration) -> Result<Self> {
        if max_bytes == 0 {
            return Err(Error::new(
                ErrorKind::InvalidRequest,
                "decode budget must be at least 1 byte",
            ));
        }
        Ok(Self {
            max_bytes,
            wait,
            state: Mutex::default(),
            released: Condvar::new(),
        })
    }

    /// The process-wide budget used by sessions that are not given their own.
    /// Created on first use from [`DECODE_BUDGET_BYTES_ENV`] and
    /// [`DECODE_BUDGET_WAIT_MS_ENV`] (defaults when unset); an invalid value
    /// is an `InvalidRequest` error here and when sessions are opened.
    pub fn global() -> Result<Arc<Self>> {
        Ok(lock(global_slot()?).clone())
    }

    /// Replace the process-wide budget. Sessions opened afterwards use it;
    /// sessions already open keep the budget they hold.
    pub fn set_global(budget: Arc<Self>) -> Result<()> {
        *lock(global_slot()?) = budget;
        Ok(())
    }

    fn from_env(bytes: Option<&str>, wait_ms: Option<&str>) -> std::result::Result<Self, String> {
        let parse = |name: &str, value: &str| {
            value
                .trim()
                .parse::<u64>()
                .map_err(|_| format!("{name} must be a whole number, not {value:?}"))
        };
        let max_bytes = match bytes {
            Some(value) => parse(DECODE_BUDGET_BYTES_ENV, value)?,
            None => DEFAULT_DECODE_BUDGET_BYTES,
        };
        if max_bytes == 0 {
            return Err(format!("{DECODE_BUDGET_BYTES_ENV} must be at least 1"));
        }
        let wait = match wait_ms {
            Some(value) => Duration::from_millis(parse(DECODE_BUDGET_WAIT_MS_ENV, value)?),
            None => DEFAULT_DECODE_BUDGET_WAIT,
        };
        Self::new(max_bytes, wait).map_err(|e| e.to_string())
    }

    /// The budget, in bytes.
    pub fn max_bytes(&self) -> u64 {
        self.max_bytes
    }

    /// Longest wait for budget before a read is refused.
    pub fn wait(&self) -> Duration {
        self.wait
    }

    pub fn stats(&self) -> DecodeBudgetStats {
        let state = lock(&self.state);
        DecodeBudgetStats {
            max_bytes: self.max_bytes,
            wait: self.wait,
            in_flight_bytes: state.in_flight,
            peak_in_flight_bytes: state.peak,
            waits: state.waits,
            refusals: state.refusals,
        }
    }

    /// Reserve `bytes` until the returned guard is dropped, waiting for at
    /// most [`Self::wait`] when `may_wait`. The error is the refusal message.
    pub(crate) fn reserve(
        self: &Arc<Self>,
        bytes: u64,
        may_wait: bool,
    ) -> std::result::Result<Reservation, String> {
        if bytes == 0 {
            return Ok(Reservation {
                budget: self.clone(),
                bytes: 0,
            });
        }
        let mut state = lock(&self.state);
        if bytes > self.max_bytes {
            state.refusals += 1;
            return Err(format!(
                "read needs {bytes} bytes of decode memory, more than the {}-byte in-flight decode budget",
                self.max_bytes
            ));
        }
        if self.max_bytes - state.in_flight < bytes && !may_wait {
            state.refusals += 1;
            return Err(format!(
                "read needs {bytes} bytes of decode memory; {} of the {}-byte in-flight decode budget is in use, and reads on decode worker threads do not wait",
                state.in_flight, self.max_bytes
            ));
        }
        if self.max_bytes - state.in_flight < bytes {
            state.waits += 1;
            let deadline = Instant::now() + self.wait;
            while self.max_bytes - state.in_flight < bytes {
                let now = Instant::now();
                if now >= deadline {
                    state.refusals += 1;
                    return Err(format!(
                        "read needs {bytes} bytes of decode memory; {} of the {}-byte in-flight decode budget stayed in use for {} ms",
                        state.in_flight,
                        self.max_bytes,
                        self.wait.as_millis()
                    ));
                }
                state = self
                    .released
                    .wait_timeout(state, deadline - now)
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
            }
        }
        state.in_flight += bytes;
        state.peak = state.peak.max(state.in_flight);
        Ok(Reservation {
            budget: self.clone(),
            bytes,
        })
    }
}

/// The process-wide slot, created from the environment on first use.
fn global_slot() -> Result<&'static Mutex<Arc<DecodeBudget>>> {
    static GLOBAL: OnceLock<std::result::Result<Mutex<Arc<DecodeBudget>>, String>> =
        OnceLock::new();
    GLOBAL
        .get_or_init(|| {
            DecodeBudget::from_env(
                std::env::var(DECODE_BUDGET_BYTES_ENV).ok().as_deref(),
                std::env::var(DECODE_BUDGET_WAIT_MS_ENV).ok().as_deref(),
            )
            .map(|budget| Mutex::new(Arc::new(budget)))
        })
        .as_ref()
        .map_err(|message| Error::new(ErrorKind::InvalidRequest, message.clone()))
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // The state is updated in single statements; a panic elsewhere cannot
    // leave it half-written.
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// Budget held by a read in progress; released on drop.
#[derive(Debug)]
pub(crate) struct Reservation {
    budget: Arc<DecodeBudget>,
    bytes: u64,
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if self.bytes == 0 {
            return;
        }
        let mut state = lock(&self.budget.state);
        state.in_flight -= self.bytes;
        drop(state);
        self.budget.released.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Barrier;

    fn budget(max_bytes: u64, wait_ms: u64) -> Arc<DecodeBudget> {
        Arc::new(DecodeBudget::new(max_bytes, Duration::from_millis(wait_ms)).unwrap())
    }

    #[test]
    fn configuration() {
        assert_eq!(
            DecodeBudget::new(0, DEFAULT_DECODE_BUDGET_WAIT)
                .unwrap_err()
                .kind,
            ErrorKind::InvalidRequest
        );
        let defaults = DecodeBudget::from_env(None, None).unwrap();
        assert_eq!(defaults.max_bytes(), DEFAULT_DECODE_BUDGET_BYTES);
        assert_eq!(defaults.wait(), DEFAULT_DECODE_BUDGET_WAIT);
        let set = DecodeBudget::from_env(Some(" 1048576 "), Some("250")).unwrap();
        assert_eq!(set.max_bytes(), 1 << 20);
        assert_eq!(set.wait(), Duration::from_millis(250));
        for (bytes, wait) in [
            (Some("0"), None),
            (Some("4GiB"), None),
            (Some("-1"), None),
            (None, Some("1.5")),
        ] {
            assert!(
                DecodeBudget::from_env(bytes, wait).is_err(),
                "{bytes:?} {wait:?}"
            );
        }
    }

    #[test]
    fn reservations_are_released_on_drop() {
        let b = budget(100, 0);
        let first = b.reserve(60, true).unwrap();
        let second = b.reserve(40, true).unwrap();
        assert_eq!(b.stats().in_flight_bytes, 100);
        drop(first);
        assert_eq!(b.stats().in_flight_bytes, 40);
        drop(second);
        let stats = b.stats();
        assert_eq!(
            (stats.in_flight_bytes, stats.peak_in_flight_bytes),
            (0, 100)
        );
        assert_eq!((stats.waits, stats.refusals), (0, 0));
        // Empty reads reserve nothing, even from a full budget.
        let _full = b.reserve(100, true).unwrap();
        let _empty = b.reserve(0, true).unwrap();
    }

    #[test]
    fn a_read_larger_than_the_budget_is_refused_without_waiting() {
        let b = budget(100, 60_000);
        let started = Instant::now();
        let message = b.reserve(101, true).unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(message.contains("101 bytes"), "{message}");
        assert!(
            message.contains("100-byte in-flight decode budget"),
            "{message}"
        );
        assert_eq!(b.stats().refusals, 1);
        assert_eq!(b.stats().in_flight_bytes, 0);
    }

    #[test]
    fn a_read_waits_for_budget_released_by_another_thread() {
        let b = budget(100, 60_000);
        let held = b.reserve(80, true).unwrap();
        let waiting = AtomicBool::new(false);
        std::thread::scope(|scope| {
            let reader = scope.spawn(|| {
                waiting.store(true, Ordering::SeqCst);
                let reservation = b.reserve(50, true).unwrap();
                assert!(b.stats().in_flight_bytes <= 100);
                drop(reservation);
            });
            while !waiting.load(Ordering::SeqCst) || b.stats().waits == 0 {
                std::thread::yield_now();
            }
            assert_eq!(b.stats().in_flight_bytes, 80);
            drop(held);
            reader.join().unwrap();
        });
        let stats = b.stats();
        assert_eq!(
            (stats.in_flight_bytes, stats.waits, stats.refusals),
            (0, 1, 0)
        );
        assert_eq!(stats.peak_in_flight_bytes, 80);
    }

    #[test]
    fn a_read_is_refused_when_the_wait_runs_out() {
        let b = budget(100, 50);
        let _held = b.reserve(80, true).unwrap();
        let started = Instant::now();
        let message = b.reserve(50, true).unwrap_err();
        assert!(started.elapsed() >= Duration::from_millis(50));
        assert!(message.contains("80 of the 100-byte"), "{message}");
        let stats = b.stats();
        assert_eq!(
            (stats.in_flight_bytes, stats.waits, stats.refusals),
            (80, 1, 1)
        );
    }

    #[test]
    fn a_read_that_may_not_wait_is_refused_at_once() {
        let b = budget(100, 60_000);
        let held = b.reserve(80, true).unwrap();
        let started = Instant::now();
        let message = b.reserve(50, false).unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(message.contains("do not wait"), "{message}");
        assert!(b.reserve(20, false).is_ok(), "fits without waiting");
        drop(held);
        assert!(b.reserve(50, false).is_ok());
        let stats = b.stats();
        assert_eq!(
            (stats.waits, stats.refusals, stats.in_flight_bytes),
            (0, 1, 0)
        );
    }

    #[test]
    fn concurrent_reads_never_exceed_the_budget() {
        let b = budget(1_000, 60_000);
        let threads = 16;
        let start = Barrier::new(threads);
        std::thread::scope(|scope| {
            for t in 0..threads {
                let (b, start) = (&b, &start);
                scope.spawn(move || {
                    start.wait();
                    for i in 0..200u64 {
                        let bytes = 1 + (t as u64 * 7919 + i * 104_729) % 400;
                        let reservation = b.reserve(bytes, true).unwrap();
                        assert!(b.stats().in_flight_bytes <= 1_000);
                        if i % 16 == 0 {
                            std::thread::sleep(Duration::from_micros(200));
                        }
                        drop(reservation);
                    }
                });
            }
        });
        let stats = b.stats();
        assert_eq!((stats.in_flight_bytes, stats.refusals), (0, 0));
        assert!(stats.peak_in_flight_bytes <= 1_000);
    }
}
