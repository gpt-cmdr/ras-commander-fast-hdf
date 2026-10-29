//! Reads through sessions draw on the in-flight decode budget
//! (src/hdf/decode_budget.rs): concurrent reads stay within it and return the
//! same values, and a read over it is a typed `resource_limit` refusal.
//! Fixture: tests/fixtures/reader_policy/ea_latest.h5, "data" f32 (50, 101),
//! chunks (1, 101), values row * 101 + col + 1.
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/reader_policy/ea_latest.h5")
}

fn values(read: &NumericArray) -> Vec<f32> {
    match &read.values {
        NumericValues::F32(v) => v.clone(),
        other => panic!("expected f32 values, got {other:?}"),
    }
}

fn expected() -> Vec<f32> {
    (0..50 * 101).map(|i| (i + 1) as f32).collect()
}

/// A whole read of "data": 50 * 101 f32 values plus a stored and a decoded
/// 404-byte chunk for its one worker (sessions decode reads under 64 KiB
/// serially).
const WHOLE_READ: u64 = 50 * 101 * 4 + 2 * 404;

fn budget(max_bytes: u64, wait: Duration) -> Arc<DecodeBudget> {
    Arc::new(DecodeBudget::new(max_bytes, wait).unwrap())
}

fn is_resource_limit(e: &Error) -> bool {
    e.kind == ErrorKind::Unsupported && e.code == Some(UnsupportedCode::ResourceLimit)
}

#[test]
fn sessions_use_the_process_wide_budget_by_default() {
    let s = ReadOnlyHdfSession::open(fixture()).unwrap();
    assert!(Arc::ptr_eq(
        &s.decode_budget(),
        &DecodeBudget::global().unwrap()
    ));
    let own = budget(1 << 20, Duration::ZERO);
    s.set_decode_budget(own.clone());
    assert!(Arc::ptr_eq(&s.decode_budget(), &own));
    assert_eq!(values(&s.read_numeric("data", None).unwrap()), expected());
    let stats = own.stats();
    assert_eq!(stats.in_flight_bytes, 0);
    assert!(stats.peak_in_flight_bytes >= 50 * 101 * 4);
}

#[test]
fn a_read_over_the_budget_is_a_resource_limit_refusal() {
    let s = ReadOnlyHdfSession::with_options(fixture(), 4, true).unwrap();
    let b = budget(WHOLE_READ - 1, Duration::from_secs(60));
    s.set_decode_budget(b.clone());
    let started = Instant::now();
    let e = s.read_numeric("data", None).unwrap_err();
    assert!(is_resource_limit(&e), "{e:?}");
    assert!(
        e.message.contains("in-flight decode budget"),
        "{}",
        e.message
    );
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "refused without waiting"
    );
    // A smaller selection of the same dataset still reads.
    let row = s
        .read_numeric("data", Some(&[Slice::new(7, 8, 1), Slice::new(0, 101, 1)]))
        .unwrap();
    assert_eq!(values(&row), expected()[7 * 101..8 * 101]);
    let stats = b.stats();
    assert_eq!((stats.refusals, stats.in_flight_bytes), (1, 0));
    // A budget that fits the read admits it.
    s.set_decode_budget(budget(WHOLE_READ, Duration::ZERO));
    assert_eq!(values(&s.read_numeric("data", None).unwrap()), expected());
}

#[test]
fn a_read_that_waits_too_long_is_a_resource_limit_refusal() {
    let s = ReadOnlyHdfSession::with_options(fixture(), 4, true).unwrap();
    let b = budget(WHOLE_READ, Duration::from_millis(50));
    s.set_decode_budget(b.clone());
    let held = b.reserve(1, true).unwrap();
    let e = s.read_numeric("data", None).unwrap_err();
    assert!(is_resource_limit(&e), "{e:?}");
    assert!(e.message.contains("stayed in use"), "{}", e.message);
    drop(held);
    assert_eq!(values(&s.read_numeric("data", None).unwrap()), expected());
    let stats = b.stats();
    assert_eq!(
        (stats.waits, stats.refusals, stats.in_flight_bytes),
        (1, 1, 0)
    );
}

#[test]
fn concurrent_reads_wait_for_the_shared_budget_and_return_the_same_values() {
    // Room for one whole read at a time, so concurrent whole reads queue.
    let b = budget(WHOLE_READ + WHOLE_READ / 2, Duration::from_secs(120));
    let shared = ReadOnlyHdfSession::with_options(fixture(), 4, true).unwrap();
    shared.set_decode_budget(b.clone());
    let want = expected();
    std::thread::scope(|scope| {
        for t in 0..8 {
            let (b, shared, want) = (&b, &shared, &want);
            scope.spawn(move || {
                // Half the threads share one session, half open their own.
                let own;
                let s = if t % 2 == 0 {
                    shared
                } else {
                    own = ReadOnlyHdfSession::with_options(fixture(), 4, true).unwrap();
                    own.set_decode_budget(b.clone());
                    &own
                };
                for i in 0..20u64 {
                    if (t + i) % 3 == 0 {
                        let rows = (i % 50, i % 50 + 1);
                        let row = s
                            .read_numeric(
                                "data",
                                Some(&[Slice::new(rows.0, rows.1, 1), Slice::new(0, 101, 1)]),
                            )
                            .unwrap();
                        let start = (rows.0 * 101) as usize;
                        assert_eq!(values(&row), want[start..start + 101]);
                    } else {
                        assert_eq!(values(&s.read_numeric("data", None).unwrap()), *want);
                    }
                    assert!(b.stats().in_flight_bytes <= b.max_bytes());
                }
            });
        }
    });
    let stats = b.stats();
    assert_eq!((stats.in_flight_bytes, stats.refusals), (0, 0));
    assert!(stats.peak_in_flight_bytes <= b.max_bytes());
    assert!(stats.peak_in_flight_bytes >= WHOLE_READ);
}

#[test]
fn parallel_reads_on_pool_workers_are_admitted_or_refused_never_blocked() {
    // ea_deep.h5: "data" f32 (9000, 2), chunks (1, 2), values row * 2 + col + 1.
    // 72,000 bytes, so 4-thread sessions decode it in parallel inside their
    // pool: the estimate adds a stored and a decoded 8-byte chunk for each of
    // 4 workers, and the reservation is taken on a pool worker, which never
    // waits (a blocked worker could deadlock the pool).
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/reader_policy/ea_deep.h5");
    let whole: u64 = 9000 * 2 * 4 + 4 * 2 * 8;
    let want: Vec<f32> = (0..9000 * 2).map(|i| (i + 1) as f32).collect();
    let run = |b: &Arc<DecodeBudget>| {
        let (refused, read) = (AtomicU64::new(0), AtomicU64::new(0));
        let started = Instant::now();
        std::thread::scope(|scope| {
            for _ in 0..6 {
                let (b, path, want, refused, read) = (b, &path, &want, &refused, &read);
                scope.spawn(move || {
                    let s = ReadOnlyHdfSession::with_options(path, 4, true).unwrap();
                    s.set_decode_budget(b.clone());
                    for _ in 0..10 {
                        match s.read_numeric("data", None) {
                            Ok(r) => {
                                assert_eq!(values(&r), *want);
                                read.fetch_add(1, Ordering::Relaxed);
                            }
                            Err(e) => {
                                assert!(is_resource_limit(&e), "{e:?}");
                                assert!(e.message.contains("do not wait"), "{}", e.message);
                                refused.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                });
            }
        });
        // Far below the 120 s wait: nothing blocked.
        assert!(started.elapsed() < Duration::from_secs(60));
        (read.into_inner(), refused.into_inner())
    };
    // Room for one read at a time: every read is either served or refused.
    let tight = budget(whole + whole / 2, Duration::from_secs(120));
    let (read, refused) = run(&tight);
    let stats = tight.stats();
    assert_eq!(read + refused, 60);
    assert!(read >= 1);
    assert_eq!(
        (stats.refusals, stats.waits, stats.in_flight_bytes),
        (refused, 0, 0)
    );
    assert_eq!(
        stats.peak_in_flight_bytes, whole,
        "one parallel read at a time"
    );
    // Room for all six: every read is served.
    let roomy = budget(6 * whole, Duration::from_secs(120));
    assert_eq!(run(&roomy), (60, 0));
    assert!(roomy.stats().peak_in_flight_bytes <= 6 * whole);
    // Less budget than one parallel read: refused.
    let s = ReadOnlyHdfSession::with_options(&path, 4, true).unwrap();
    s.set_decode_budget(budget(whole - 1, Duration::ZERO));
    assert!(is_resource_limit(
        &s.read_numeric("data", None).unwrap_err()
    ));
}
