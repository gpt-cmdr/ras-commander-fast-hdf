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
//!
//! Admission and release are lock-free: a read reserves with one
//! compare-and-swap on the in-flight count, and a release subtracts and takes
//! the lock only when a read is waiting. The lock and condition variable are
//! used only by reads that wait. A waiter registers itself, then re-checks the
//! budget under the lock before it sleeps; a release subtracts, then checks for
//! waiters and notifies under the lock. With both sides sequentially
//! consistent, either the release sees the waiter (and its notify cannot be
//! lost, because the waiter holds the lock from its re-check until it sleeps)
//! or the waiter's re-check sees the released bytes.
use crate::{Error, ErrorKind, Result};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::SeqCst};
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
    in_flight: AtomicU64,
    peak: AtomicU64,
    waits: AtomicU64,
    refusals: AtomicU64,
    /// Reads registered to wait; a release notifies only when this is nonzero.
    waiters: AtomicUsize,
    /// Held by a waiter from its re-check until it sleeps, and by a release
    /// before it notifies.
    lock: Mutex<()>,
    released: Condvar,
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
            in_flight: AtomicU64::new(0),
            peak: AtomicU64::new(0),
            waits: AtomicU64::new(0),
            refusals: AtomicU64::new(0),
            waiters: AtomicUsize::new(0),
            lock: Mutex::new(()),
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

    /// A snapshot of the budget. Each field is read on its own, so under
    /// concurrent reads the fields may come from slightly different moments.
    pub fn stats(&self) -> DecodeBudgetStats {
        DecodeBudgetStats {
            max_bytes: self.max_bytes,
            wait: self.wait,
            in_flight_bytes: self.in_flight.load(SeqCst),
            peak_in_flight_bytes: self.peak.load(SeqCst),
            waits: self.waits.load(SeqCst),
            refusals: self.refusals.load(SeqCst),
        }
    }

    /// Add `bytes` to the in-flight count if the budget has room for them.
    /// The count never exceeds `max_bytes`; without room, the count seen.
    fn try_add(&self, bytes: u64) -> std::result::Result<(), u64> {
        let mut current = self.in_flight.load(SeqCst);
        loop {
            if self.max_bytes - current < bytes {
                return Err(current);
            }
            match self
                .in_flight
                .compare_exchange_weak(current, current + bytes, SeqCst, SeqCst)
            {
                Ok(_) => {
                    self.peak.fetch_max(current + bytes, SeqCst);
                    return Ok(());
                }
                Err(seen) => current = seen,
            }
        }
    }

    fn refuse(&self, message: String) -> std::result::Result<Reservation, String> {
        self.refusals.fetch_add(1, SeqCst);
        Err(message)
    }

    /// Reserve `bytes` until the returned guard is dropped, waiting for at
    /// most [`Self::wait`] when `may_wait`. The error is the refusal message.
    pub(crate) fn reserve(
        self: &Arc<Self>,
        bytes: u64,
        may_wait: bool,
    ) -> std::result::Result<Reservation, String> {
        let reservation = || Reservation {
            budget: self.clone(),
            bytes,
        };
        if bytes == 0 {
            return Ok(reservation());
        }
        if bytes > self.max_bytes {
            return self.refuse(format!(
                "read needs {bytes} bytes of decode memory, more than the {}-byte in-flight decode budget",
                self.max_bytes
            ));
        }
        let in_flight = match self.try_add(bytes) {
            Ok(()) => return Ok(reservation()),
            Err(in_flight) => in_flight,
        };
        if !may_wait {
            return self.refuse(format!(
                "read needs {bytes} bytes of decode memory; {in_flight} of the {}-byte in-flight decode budget is in use, and reads on decode worker threads do not wait",
                self.max_bytes
            ));
        }
        self.waits.fetch_add(1, SeqCst);
        let deadline = Instant::now() + self.wait;
        let mut guard = lock(&self.lock);
        // Registered before the re-check below and removed on every exit,
        // including a panic, so a release never skips a sleeping waiter.
        let _waiting = Waiting::register(&self.waiters);
        loop {
            let in_flight = match self.try_add(bytes) {
                Ok(()) => return Ok(reservation()),
                Err(in_flight) => in_flight,
            };
            let now = Instant::now();
            if now >= deadline {
                drop(guard);
                return self.refuse(format!(
                    "read needs {bytes} bytes of decode memory; {in_flight} of the {}-byte in-flight decode budget stayed in use for {} ms",
                    self.max_bytes,
                    self.wait.as_millis()
                ));
            }
            guard = self
                .released
                .wait_timeout(guard, deadline - now)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }

    fn release(&self, bytes: u64) {
        self.in_flight.fetch_sub(bytes, SeqCst);
        if self.waiters.load(SeqCst) != 0 {
            // A waiter that registered before this load holds the lock from
            // its re-check until it sleeps, so taking the lock here orders the
            // notify after that sleep; a later waiter's re-check sees the bytes.
            drop(lock(&self.lock));
            self.released.notify_all();
        }
    }
}

/// A waiter's registration in [`DecodeBudget::waiters`], removed on drop.
struct Waiting<'a>(&'a AtomicUsize);

impl<'a> Waiting<'a> {
    fn register(waiters: &'a AtomicUsize) -> Self {
        waiters.fetch_add(1, SeqCst);
        Self(waiters)
    }
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, SeqCst);
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
    // The budget's lock guards no data, and the global slot is replaced in a
    // single statement; a panic elsewhere cannot leave either half-written.
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
        if self.bytes != 0 {
            self.budget.release(self.bytes);
        }
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

    /// Run `f` on its own thread; fail if it does not finish within `limit`
    /// (a deadlock or a lost wakeup would otherwise hang the test run).
    fn within<T: Send + 'static>(limit: Duration, f: impl FnOnce() -> T + Send + 'static) -> T {
        let (done, finished) = std::sync::mpsc::channel();
        std::thread::spawn(move || done.send(f()).unwrap());
        finished
            .recv_timeout(limit)
            .expect("did not finish in time: deadlock or lost wakeup")
    }

    #[test]
    fn stress_many_threads_over_a_small_budget_never_exceed_it_and_end_at_zero() {
        // 32 threads asking for up to 400 bytes each (about 6.4 kB combined)
        // from a 1,000-byte budget, mixing waiting reads and reads that may
        // not wait. An independent counter checks the cap at every moment.
        const MAX: u64 = 1_000;
        let b = budget(MAX, 60_000);
        let held = Arc::new(AtomicU64::new(0));
        let refused_no_wait = Arc::new(AtomicU64::new(0));
        let (b2, held2, refused2) = (b.clone(), held.clone(), refused_no_wait.clone());
        within(Duration::from_secs(120), move || {
            let threads = 32;
            let start = Barrier::new(threads);
            std::thread::scope(|scope| {
                for t in 0..threads {
                    let (b, held, refused, start) = (&b2, &held2, &refused2, &start);
                    scope.spawn(move || {
                        start.wait();
                        for i in 0..2_000u64 {
                            let bytes = 1 + (t as u64 * 7_919 + i * 104_729) % 400;
                            let may_wait = !(t + i as usize).is_multiple_of(5);
                            match b.reserve(bytes, may_wait) {
                                Ok(reservation) => {
                                    let now = held.fetch_add(bytes, SeqCst) + bytes;
                                    assert!(now <= MAX, "{now} bytes held");
                                    if i % 64 == 0 {
                                        std::thread::sleep(Duration::from_micros(100));
                                    } else {
                                        std::hint::spin_loop();
                                    }
                                    held.fetch_sub(bytes, SeqCst);
                                    drop(reservation);
                                }
                                Err(message) => {
                                    assert!(!may_wait, "a waiting read was refused: {message}");
                                    refused.fetch_add(1, SeqCst);
                                }
                            }
                        }
                    });
                }
            });
        });
        let stats = b.stats();
        assert_eq!(stats.in_flight_bytes, 0);
        assert_eq!(b.waiters.load(SeqCst), 0);
        assert_eq!(held.load(SeqCst), 0);
        assert!(stats.peak_in_flight_bytes <= MAX);
        assert_eq!(stats.refusals, refused_no_wait.load(SeqCst));
        assert!(stats.waits > 0, "the budget was never contended");
    }

    #[test]
    fn a_release_always_wakes_the_waiter() {
        // A one-byte budget passed back and forth between threads that hold
        // it for a few microseconds, so the others go to sleep and only a
        // release can wake them. A lost wakeup leaves a thread asleep until its
        // 60 s wait runs out, past the 30 s limit.
        let b = budget(1, 60_000);
        let b2 = b.clone();
        within(Duration::from_secs(30), move || {
            std::thread::scope(|scope| {
                for _ in 0..4 {
                    let b = &b2;
                    scope.spawn(move || {
                        for _ in 0..2_000 {
                            let reservation = b.reserve(1, true).unwrap();
                            let held = Instant::now();
                            while held.elapsed() < Duration::from_micros(5) {
                                std::hint::spin_loop();
                            }
                            drop(reservation);
                        }
                    });
                }
            });
        });
        let stats = b.stats();
        assert_eq!((stats.in_flight_bytes, stats.refusals), (0, 0));
        assert_eq!(b.waiters.load(SeqCst), 0);
        assert!(stats.waits > 0, "no read ever waited");
    }

    #[test]
    fn a_panicking_read_returns_its_reservation_and_wakes_waiters() {
        let b = budget(100, 60_000);
        let holding = Arc::new(Barrier::new(2));
        let (b2, holding2) = (b.clone(), holding.clone());
        let holder = std::thread::spawn(move || {
            let _reservation = b2.reserve(100, true).unwrap();
            holding2.wait();
            // Give the waiter time to go to sleep on the full budget.
            while b2.waiters.load(SeqCst) == 0 {
                std::thread::yield_now();
            }
            panic!("read failed while holding its reservation");
        });
        holding.wait();
        let b3 = b.clone();
        within(Duration::from_secs(30), move || {
            drop(b3.reserve(100, true).unwrap());
        });
        assert!(holder.join().is_err(), "the holder panicked");
        let stats = b.stats();
        assert_eq!((stats.in_flight_bytes, stats.refusals), (0, 0));
        assert_eq!(b.waiters.load(SeqCst), 0);
        assert!(stats.waits >= 1);
    }
}
