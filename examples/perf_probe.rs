//! JSON-lines measurement server. No engine instrumentation or semantic changes.
use hdf5_reader::{H5Type, Hdf5File, SliceInfo, SliceInfoElem};
use ras_commander_fast_hdf::hdf::{
    mesh_geometry::MeshGeometryField, NumericArray, NumericValues, ReadOnlyHdfSession, Slice,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::{
    io::{self, BufRead, Write},
    time::Instant,
};
struct TimedStorage {
    inner: hdf5_reader::FileStorage,
    nanos: AtomicU64,
    count: AtomicU64,
    bytes: AtomicU64,
}
impl hdf5_reader::Storage for TimedStorage {
    fn len(&self) -> u64 {
        hdf5_reader::Storage::len(&self.inner)
    }
    fn read_range(
        &self,
        offset: u64,
        len: usize,
    ) -> hdf5_reader::error::Result<hdf5_reader::StorageBuffer> {
        let t = Instant::now();
        let result = hdf5_reader::Storage::read_range(&self.inner, offset, len);
        self.nanos
            .fetch_add(t.elapsed().as_nanos() as u64, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);
        self.bytes.fetch_add(len as u64, Ordering::Relaxed);
        result
    }
}
const META: usize = 128 * 1024 * 1024;
const MAX: usize = 8 * 1024 * 1024 * 1024;
#[derive(Deserialize)]
struct Request {
    file: String,
    mode: String,
    workload: String,
    area: String,
    paths: Vec<String>,
    dtypes: Vec<String>,
    rows: u64,
    elements: Option<Vec<u64>>,
}
fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.
}
fn bytes(a: NumericArray) -> Vec<u8> {
    macro_rules! b {
        ($v:expr) => {
            $v.iter().flat_map(|x| x.to_le_bytes()).collect()
        };
    }
    match a.values {
        NumericValues::F32(v) => b!(v),
        NumericValues::F64(v) => b!(v),
        NumericValues::I32(v) => b!(v),
        NumericValues::I64(v) => b!(v),
        NumericValues::U32(v) => b!(v),
        NumericValues::U64(v) => b!(v),
        NumericValues::I16(v) => b!(v),
        NumericValues::U16(v) => b!(v),
        NumericValues::I8(v) => b!(v),
        NumericValues::U8(v) => v,
        _ => panic!("unhandled numeric type"),
    }
}
fn engine(
    s: &ReadOnlyHdfSession,
    q: &Request,
) -> Result<Vec<NumericArray>, Box<dyn std::error::Error>> {
    Ok(match q.workload.as_str() {
        "M1" => {
            std::hint::black_box(s.layer_inventory(META)?);
            vec![]
        }
        "P1" => vec![
            s.read_mesh_summary(
                &q.area,
                q.paths[0].rsplit('/').next().unwrap(),
                q.elements.as_deref(),
                META,
                MAX,
            )?
            .values,
        ],
        "B3" => [
            MeshGeometryField::CellCenters,
            MeshGeometryField::FaceCellIndexes,
            MeshGeometryField::FacePoints,
        ]
        .into_iter()
        .map(|f| s.read_mesh_geometry(&q.area, f, None, META, MAX))
        .collect::<Result<Vec<_>, _>>()?,
        _ => q
            .paths
            .iter()
            .map(|p| {
                s.read_mesh_series(
                    &q.area,
                    p.rsplit('/').next().unwrap(),
                    Slice::new(0, q.rows, 1),
                    q.elements.as_deref(),
                    META,
                    MAX,
                )
                .map(|v| v.values)
            })
            .collect::<Result<Vec<_>, _>>()?,
    })
}
fn raw_t<T: H5Type + Default + Copy>(
    file: &Hdf5File,
    p: &str,
    q: &Request,
    lookup: &mut f64,
    read: &mut f64,
    gather: &mut f64,
) -> Result<Vec<T>, Box<dyn std::error::Error>> {
    let t = Instant::now();
    let ds = file.dataset(p)?;
    *lookup += ms(t);
    let rank = ds.ndim();
    let nr = if rank == 1 {
        1
    } else if q.workload == "B3" {
        ds.shape()[0]
    } else {
        q.rows
    };
    let n = *ds.shape().last().unwrap();
    let elems = q.elements.clone().unwrap_or_else(|| (0..n).collect());
    let contiguous = elems.windows(2).all(|w| w[1] == w[0] + 1);
    let slab = |lo: u64, hi: u64| {
        let mut sels = vec![];
        if rank > 1 {
            sels.push(SliceInfoElem::Slice {
                start: 0,
                end: nr,
                step: 1,
            });
        }
        sels.push(SliceInfoElem::Slice {
            start: lo,
            end: hi,
            step: 1,
        });
        ds.read_slice::<T>(&SliceInfo { selections: sels })
    };
    if contiguous {
        let t = Instant::now();
        let a = slab(elems[0], elems[elems.len() - 1] + 1)?;
        *read += ms(t);
        return Ok(a.into_raw_vec_and_offset().0);
    }
    let mut out = vec![T::default(); nr as usize * elems.len()];
    let chunk = ds.chunks().and_then(|v| v.last().copied());
    let mut at = 0;
    while at < elems.len() {
        let start = at;
        let mut lo = elems[at];
        let mut hi = lo;
        at += 1;
        while at < elems.len() {
            let l = lo.min(elems[at]);
            let h = hi.max(elems[at]);
            if !chunk.is_some_and(|c| l / u64::from(c) == h / u64::from(c)) {
                break;
            }
            lo = l;
            hi = h;
            at += 1;
        }
        let t = Instant::now();
        let a = slab(lo, hi + 1)?;
        *read += ms(t);
        let t = Instant::now();
        let v = a.as_slice().unwrap();
        let span = (hi - lo + 1) as usize;
        for r in 0..nr as usize {
            for c in start..at {
                out[r * elems.len() + c] = v[r * span + (elems[c] - lo) as usize];
            }
        }
        *gather += ms(t);
    }
    Ok(out)
}
fn run(
    q: &Request,
    session: &mut Option<(String, ReadOnlyHdfSession)>,
    raw: &mut Option<(String, Hdf5File, Arc<TimedStorage>)>,
) -> Result<Value, Box<dyn std::error::Error>> {
    let mut lookup = 0.;
    let mut read = 0.;
    let mut gather = 0.;
    let mut inventory = None;
    let mut cache = Value::Null;
    let t = Instant::now();
    let (open, elapsed, arrays, raw_bytes) = if q.mode == "raw" {
        if raw.as_ref().is_none_or(|(p, _, _)| p != &q.file) {
            let storage = Arc::new(TimedStorage {
                inner: hdf5_reader::FileStorage::open(&q.file)?,
                nanos: AtomicU64::new(0),
                count: AtomicU64::new(0),
                bytes: AtomicU64::new(0),
            });
            let file = Hdf5File::from_storage(storage.clone())?;
            *raw = Some((q.file.clone(), file, storage));
        }
        let open = ms(t);
        let f = &raw.as_ref().unwrap().1;
        let before = f.chunk_cache_stats();
        let storage = &raw.as_ref().unwrap().2;
        let io_before = (
            storage.nanos.load(Ordering::Relaxed),
            storage.count.load(Ordering::Relaxed),
            storage.bytes.load(Ordering::Relaxed),
        );
        let t = Instant::now();
        let mut outputs = vec![];
        let mut serialization_ms = 0.;
        for (p, d) in q.paths.iter().zip(&q.dtypes) {
            macro_rules! rd {
                ($ty:ty) => {{
                    let v = raw_t::<$ty>(f, p, q, &mut lookup, &mut read, &mut gather)?;
                    {
                        let t = Instant::now();
                        let b = v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
                        serialization_ms += ms(t);
                        b
                    }
                }};
            }
            outputs.push(match d.as_str() {
                "<f4" => rd!(f32),
                "<f8" => rd!(f64),
                "<i4" => rd!(i32),
                "<i8" => rd!(i64),
                "<u4" => rd!(u32),
                "<u8" => rd!(u64),
                _ => return Err(format!("raw dtype {d}").into()),
            });
        }
        let elapsed = ms(t) - serialization_ms;
        let after = f.chunk_cache_stats();
        cache = json!({"io_ms":(storage.nanos.load(Ordering::Relaxed)-io_before.0) as f64/1e6,"io_ranges":storage.count.load(Ordering::Relaxed)-io_before.1,"io_bytes":storage.bytes.load(Ordering::Relaxed)-io_before.2,"hits":after.hits-before.hits,"misses":after.misses-before.misses,"evictions":after.evictions-before.evictions,"bytes":after.current_bytes});
        (open, elapsed, vec![], outputs)
    } else {
        let owned;
        if q.mode == "cold" {
            owned = Some(ReadOnlyHdfSession::open(&q.file)?);
        } else {
            owned = None;
            if session.as_ref().is_none_or(|(p, _)| p != &q.file) {
                *session = Some((q.file.clone(), ReadOnlyHdfSession::open(&q.file)?));
            }
        }
        let open = ms(t);
        let s = owned
            .as_ref()
            .unwrap_or_else(|| &session.as_ref().unwrap().1);
        let t = Instant::now();
        let arrays = engine(s, q)?;
        let elapsed = ms(t);
        // Separate diagnostic after the timed operation; never prime its cold session.
        let t = Instant::now();
        match q.workload.as_str() {
            "P1" => {
                std::hint::black_box(s.mesh_summary_inventory(&q.area, META)?);
            }
            "B3" => {
                std::hint::black_box(s.mesh_geometry(&q.area, META)?);
            }
            "M1" => {}
            _ => {
                std::hint::black_box(s.mesh_series_inventory(&q.area, META)?);
            }
        };
        inventory = Some(ms(t));
        (open, elapsed, arrays, vec![])
    };
    let t = Instant::now();
    let outputs = if q.mode == "raw" {
        raw_bytes
    } else {
        arrays.into_iter().map(bytes).collect()
    };
    let hashes: Vec<_> = outputs
        .iter()
        .map(|b| format!("{:x}", Sha256::digest(b)))
        .collect();
    let sizes: Vec<_> = outputs.iter().map(Vec::len).collect();
    Ok(
        json!({"mode":q.mode,"workload":q.workload,"open_ms":open,"call_ms":elapsed,"total_ms":open+elapsed,"inventory_diagnostic_ms":inventory,"lookup_ms":lookup,"read_combined_ms":read,"gather_ms":gather,"cache":cache,"hashes":hashes,"bytes":sizes,"hash_and_serialization_ms":ms(t)}),
    )
}
fn main() {
    let mut session = None;
    let mut raw = None;
    for line in io::stdin().lock().lines() {
        let line = line.unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
            || -> Result<Value, Box<dyn std::error::Error>> {
                let q: Request = serde_json::from_str(&line)?;
                run(&q, &mut session, &mut raw)
            },
        ));
        let v = match result {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => json!({"error":e.to_string()}),
            Err(_) => json!({"error":"panic"}),
        };
        println!("{v}");
        io::stdout().flush().unwrap();
    }
}
