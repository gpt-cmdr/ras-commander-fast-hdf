//! Session-local exact-key metadata memoization. No file handles or source data are global.
use super::ReadOnlyHdfSession;
use crate::{Error, ErrorKind, Result};
use std::{
    any::Any,
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};
type Cell<T> = Arc<OnceLock<Result<T>>>;
#[derive(Default)]
pub(super) struct Memo {
    values: Mutex<HashMap<String, Box<dyn Any + Send + Sync>>>,
    numeric: Mutex<HashMap<String, Arc<NumericDataset>>>,
}
pub(super) struct NumericDataset {
    pub dataset: hdf5_reader::Dataset,
    pub kind: OnceLock<super::NumericKind>,
}
impl ReadOnlyHdfSession {
    // Exact caller keys preserve aliases and error context. Cache only successful
    // metadata opens, never a host error; numeric proof still follows selection validation.
    pub(super) fn numeric_dataset(&self, path: &str) -> Result<Arc<NumericDataset>> {
        self.numeric_dataset_with(path, || self.file.dataset(path).map_err(super::map_error))
    }
    fn numeric_dataset_with(
        &self,
        path: &str,
        open: impl FnOnce() -> Result<hdf5_reader::Dataset>,
    ) -> Result<Arc<NumericDataset>> {
        {
            let cache = self.memo.numeric.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(value) = cache.get(path) {
                return Ok(value.clone());
            }
        }
        // Header parsing may block or panic. Keep it outside the shared registry lock.
        let value = Arc::new(NumericDataset {
            dataset: open()?,
            kind: OnceLock::new(),
        });
        let mut cache = self.memo.numeric.lock().unwrap_or_else(|e| e.into_inner());
        Ok(cache.entry(path.to_owned()).or_insert(value).clone())
    }
    pub(crate) fn memo<T: Clone + Send + Sync + 'static>(
        &self,
        key: String,
        compute: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        let cell = {
            let mut map = self
                .memo
                .values
                .lock()
                .map_err(|_| Error::new(ErrorKind::Internal, "memo registry poisoned"))?;
            map.entry(key.clone())
                .or_insert_with(|| Box::new(Cell::<T>::default()))
                .downcast_ref::<Cell<T>>()
                .ok_or_else(|| Error::new(ErrorKind::Internal, "memo key type mismatch"))?
                .clone()
        };
        let result = cell.get_or_init(compute).clone();
        // Host/storage failures may be transient. Never pin an error into the session.
        if result.is_err() {
            let mut map = self
                .memo
                .values
                .lock()
                .map_err(|_| Error::new(ErrorKind::Internal, "memo registry poisoned"))?;
            if map
                .get(&key)
                .and_then(|v| v.downcast_ref::<Cell<T>>())
                .is_some_and(|v| Arc::ptr_eq(v, &cell))
            {
                map.remove(&key);
            }
        }
        result
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn distinct_cold_opens_do_not_hold_the_registry_lock() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/hdf_numeric/numeric_types_v1.h5");
        let s = ReadOnlyHdfSession::open(path).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            let session = &s;
            let first = scope.spawn(move || {
                session.numeric_dataset_with("int/u64_bounds", || {
                    tx.send(()).unwrap();
                    release_rx
                        .recv_timeout(std::time::Duration::from_secs(5))
                        .unwrap();
                    session
                        .file
                        .dataset("int/u64_bounds")
                        .map_err(super::super::map_error)
                })
            });
            rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
            // A different real dataset opens while the first opener is still blocked.
            let second = s.numeric_dataset("float/f8_special");
            release_tx.send(()).unwrap();
            first.join().unwrap().unwrap();
            second.unwrap();
        });
    }
    #[test]
    fn opener_panic_and_poisoned_registry_do_not_break_later_reads() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/hdf_numeric/numeric_types_v1.h5");
        let s = ReadOnlyHdfSession::open(path).unwrap();
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = s.numeric_dataset_with("int/u64_bounds", || panic!("injected open panic"));
        }))
        .is_err());
        assert!(!s.memo.numeric.is_poisoned());
        assert!(s.read_numeric("int/u64_bounds", None).is_ok());
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = s.memo.numeric.lock().unwrap();
            panic!("injected registry poison");
        }))
        .is_err());
        assert!(s.memo.numeric.is_poisoned());
        assert!(s.read_numeric("int/u64_bounds", None).is_ok());
        assert!(s.numeric_dataset("float/f8_special").is_ok());
    }
    #[test]
    fn numeric_metadata_is_shared_without_pinning_errors_or_skipping_validation() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/hdf_numeric/numeric_types_v1.h5");
        let s = ReadOnlyHdfSession::open(path).unwrap();
        let name = String::from("int/u64_bounds");
        let a = s.numeric_dataset(&name).unwrap();
        assert!(a.kind.get().is_none());
        let bad = s
            .read_numeric(&name, Some(&[super::super::Slice::new(0, u64::MAX, 0)]))
            .unwrap_err();
        assert_eq!(bad.kind, ErrorKind::InvalidRequest);
        assert!(a.kind.get().is_none());
        let expected = s.read_numeric(&name, None).unwrap();
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    assert!(Arc::ptr_eq(&a, &s.numeric_dataset(&name).unwrap()));
                    assert_eq!(s.read_numeric(&name, None).unwrap(), expected);
                });
            }
        });
        assert!(a.kind.get().is_some());
        let before = s.memo.numeric.lock().unwrap().len();
        for _ in 0..2 {
            assert!(s.numeric_dataset("missing-dataset").is_err());
        }
        assert_eq!(s.memo.numeric.lock().unwrap().len(), before);
    }
    #[test]
    fn actual_session_memo_is_once_per_key_concurrent_and_retries_errors() {
        let s = ReadOnlyHdfSession::open(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/hdf_numeric/numeric_types_v1.h5"),
        )
        .unwrap();
        let count = std::sync::atomic::AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    let mut v = s
                        .memo("test:one".into(), || {
                            count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            Ok(vec![1u8, 2])
                        })
                        .unwrap();
                    v[0] = 9;
                });
            }
        });
        assert_eq!(count.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert_eq!(
            s.memo("test:one".into(), || Ok(vec![7u8])).unwrap(),
            vec![1, 2]
        );
        for _ in 0..2 {
            let _: Result<usize> = s.memo("test:error".into(), || {
                count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Err(Error::new(ErrorKind::Io, "retry"))
            });
        }
        assert_eq!(count.load(std::sync::atomic::Ordering::Relaxed), 3);
        assert_eq!(s.memo("test:two".into(), || Ok(2usize)).unwrap(), 2);
    }
}
