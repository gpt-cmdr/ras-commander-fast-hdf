//! Plan time axes for unsteady output blocks.
//!
//! Civil timestamps are model-local calendar values: no time zone, DST or leap-second
//! rule is invented. Integer milliseconds count from the declared [`TimeEpoch`], which
//! is itself a civil (zone-less) instant. The exact raw strings and raw integers are
//! always retained next to every derived value.
use super::{
    encoding::map_error, metadata::MetadataValue, metadata_proof as proof, xs1d::UNSTEADY,
    NumericKind, NumericValues, ReadOnlyHdfSession, Slice,
};
use crate::capability::absent_ok;
use crate::{Error, ErrorKind, Result, UnsupportedCode};
use std::fmt;

/// Group holding the base unsteady output time series.
pub const UNSTEADY_TIME_GROUP: &str = UNSTEADY;
const DAYS: &str = "Time";
const STAMP: &str = "Time Date Stamp";
const STAMP_MS: &str = "Time Date Stamp (ms)";
const MS_PER_DAY: i64 = 86_400_000;
const MONTHS: [&str; 12] = [
    "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
];

/// Declared epoch for [`TimeAxis::ms_since_epoch`]. Always a zone-less civil instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TimeEpoch {
    /// 1970-01-01T00:00:00 in the model-local proleptic Gregorian civil calendar.
    /// This is not a UTC instant claim; it is the origin used to count civil milliseconds.
    CivilUnix1970,
}
/// Where the integer milliseconds came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MillisecondSource {
    /// Stored fixed-string `Time Date Stamp (ms)` (`DDMMMYYYY HH:MM:SS:mmm`, the real
    /// HEC-RAS form), parsed and cross-checked against the other axes.
    StoredStringDataset,
    /// Derived from the parsed `Time Date Stamp` strings (no ms dataset present).
    DerivedFromStrings,
}
/// Validated civil (zone-less) calendar instant. Hour is 0..=23; 24:00 is normalized by parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub struct CivilTimestamp {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub millisecond: u16,
}
fn corrupt(m: impl Into<String>) -> Error {
    Error::new(ErrorKind::Corrupt, m)
}
fn unsupported(code: UnsupportedCode, m: impl Into<String>) -> Error {
    Error::unsupported(code, m)
}
fn leap(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}
fn days_in_month(y: i32, m: u8) -> u8 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if leap(y) => 29,
        _ => 28,
    }
}
/// Days since 1970-01-01 (proleptic Gregorian).
fn days_from_civil(y: i32, m: u8, d: u8) -> i64 {
    let y = i64::from(y) - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (i64::from(m) + 9) % 12;
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}
fn civil_from_days(z: i64) -> (i64, u8, u8) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}
impl CivilTimestamp {
    /// Validate calendar ranges (leap years included). Hour must be 0..=23. Out-of-range
    /// caller input is `InvalidRequest`; stored values that fail the same check are reported
    /// Corrupt by the readers that wrap this constructor.
    pub fn new(
        year: i32,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
        millisecond: u16,
    ) -> Result<Self> {
        if !(1..=9999).contains(&year)
            || !(1..=12).contains(&month)
            || day == 0
            || day > days_in_month(year, month)
            || hour > 23
            || minute > 59
            || second > 59
            || millisecond > 999
        {
            return Err(Error::new(
                ErrorKind::InvalidRequest,
                format!(
                    "civil timestamp out of range: {year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}.{millisecond:03}"
                ),
            ));
        }
        Ok(Self {
            year,
            month,
            day,
            hour,
            minute,
            second,
            millisecond,
        })
    }
    /// Civil milliseconds since [`TimeEpoch::CivilUnix1970`].
    pub fn ms_since_epoch(&self) -> i64 {
        days_from_civil(self.year, self.month, self.day) * MS_PER_DAY
            + (i64::from(self.hour) * 3600 + i64::from(self.minute) * 60 + i64::from(self.second))
                * 1000
            + i64::from(self.millisecond)
    }
    /// Inverse of [`Self::ms_since_epoch`]; years outside 1..=9999 are Corrupt.
    pub fn from_ms_since_epoch(ms: i64) -> Result<Self> {
        let days = ms.div_euclid(MS_PER_DAY);
        let rem = ms.rem_euclid(MS_PER_DAY);
        let (y, m, d) = civil_from_days(days);
        let year = i32::try_from(y).map_err(|_| corrupt("timestamp year out of range"))?;
        Self::new(
            year,
            m,
            d,
            (rem / 3_600_000) as u8,
            (rem / 60_000 % 60) as u8,
            (rem / 1000 % 60) as u8,
            (rem % 1000) as u16,
        )
        .map_err(|e| corrupt(e.message))
    }
}
impl fmt::Display for CivilTimestamp {
    /// ISO-like text without any zone designator.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}",
            self.year, self.month, self.day, self.hour, self.minute, self.second, self.millisecond
        )
    }
}
/// A parsed HEC stamp. `hour_2400` records that the source used 24:00:00 (rolled to next day).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ParsedStamp {
    pub timestamp: CivilTimestamp,
    pub hour_2400: bool,
}
/// Parse `DDMMMYYYY HH:MM:SS` (seconds optional), e.g. `01JAN2020 24:00:00`. Documented
/// leniency: surrounding/inner ASCII whitespace separating date and time is accepted and
/// month names are case-insensitive; field widths are otherwise exact.
/// Shape mismatches are Unsupported; impossible calendar values are Corrupt.
pub fn parse_hec_stamp(text: &str) -> Result<ParsedStamp> {
    parse_stamp(text, false)
}
/// Parse the real `Time Date Stamp (ms)` string form `DDMMMYYYY HH:MM:SS:mmm`, e.g.
/// `01JAN1999 12:00:00:000`. Field widths are exactly 2,3,4 / 2,2,2,3; `24:00:00:000`
/// rolls to the next day. Shape mismatches are Unsupported; impossible values Corrupt.
pub fn parse_hec_ms_stamp(text: &str) -> Result<ParsedStamp> {
    parse_stamp(text, true)
}
fn parse_stamp(text: &str, ms_form: bool) -> Result<ParsedStamp> {
    let shape = || {
        unsupported(
            UnsupportedCode::Encoding,
            format!("unrecognized HEC date stamp {text:?}"),
        )
    };
    let mut parts = text.split_whitespace();
    let (date, time) = (
        parts.next().ok_or_else(shape)?,
        parts.next().ok_or_else(shape)?,
    );
    if parts.next().is_some() || !date.is_ascii() || date.len() != 9 {
        return Err(shape());
    }
    let digits = |s: &str| -> Option<u32> {
        if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) {
            s.parse().ok()
        } else {
            None
        }
    };
    let day = digits(&date[0..2]).ok_or_else(shape)? as u8;
    let month = MONTHS
        .iter()
        .position(|m| m.eq_ignore_ascii_case(&date[2..5]))
        .ok_or_else(shape)? as u8
        + 1;
    let year = digits(&date[5..9]).ok_or_else(shape)? as i32;
    let fields: Vec<&str> = time.split(':').collect();
    let widths: &[usize] = if ms_form { &[2, 2, 2, 3] } else { &[2, 2, 2] };
    let count_ok = if ms_form {
        fields.len() == 4
    } else {
        (2..=3).contains(&fields.len())
    };
    if !count_ok || fields.iter().zip(widths).any(|(f, w)| f.len() != *w) {
        return Err(shape());
    }
    let mut v = [0u32; 4];
    for (i, f) in fields.iter().enumerate() {
        v[i] = digits(f).ok_or_else(shape)?;
    }
    let (h, mi, s, msec) = (v[0], v[1], v[2], v[3]);
    if h == 24 && mi == 0 && s == 0 && msec == 0 {
        // 24:00:00 is the end of the stated day, i.e. 00:00:00 of the following day.
        let base = CivilTimestamp::new(year, month, day, 0, 0, 0, 0)
            .map_err(|e| corrupt(format!("{}: {text:?}", e.message)))?;
        let next = CivilTimestamp::from_ms_since_epoch(base.ms_since_epoch() + MS_PER_DAY)?;
        return Ok(ParsedStamp {
            timestamp: next,
            hour_2400: true,
        });
    }
    if h > 23 {
        return Err(corrupt(format!("hour out of range: {text:?}")));
    }
    Ok(ParsedStamp {
        timestamp: CivilTimestamp::new(year, month, day, h as u8, mi as u8, s as u8, msec as u16)
            .map_err(|e| corrupt(format!("{}: {text:?}", e.message)))?,
        hour_2400: false,
    })
}

/// One plan time axis. Every present source dataset is retained exactly.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct TimeAxis {
    /// Group path the axis was read from.
    pub group: String,
    pub rows: u64,
    /// `Time` as stored (f32 widened exactly to f64); unit/origin are not asserted here.
    pub days: Option<Vec<f64>>,
    /// Exact decoded `Time Date Stamp` strings.
    pub raw_strings: Option<Vec<String>>,
    /// Stored `Time Date Stamp (ms)` integers (only when that dataset is integer-typed).
    pub raw_ms: Option<Vec<i64>>,
    /// Exact stored `Time Date Stamp (ms)` strings (only when that dataset is fixed text).
    pub raw_ms_strings: Option<Vec<String>>,
    /// Some(Unsupported) when the ms dataset is integer-typed: values stay in `raw_ms` but
    /// no epoch is proven, so no civil time is derived from them.
    pub integer_ms: Option<crate::Capability>,
    /// Row indices (into the axis) whose source string used hour 24.
    pub hour_2400_rows: Vec<u64>,
    pub epoch: TimeEpoch,
    pub ms_source: Option<MillisecondSource>,
    /// Integer civil milliseconds since `epoch`; stored values preferred over derived.
    pub ms_since_epoch: Option<Vec<i64>>,
    /// Typed civil timestamps matching `ms_since_epoch`.
    pub timestamps: Option<Vec<CivilTimestamp>>,
}
/// A row selection of a [`TimeAxis`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct TimeSelection {
    pub row_indices: Vec<u64>,
    pub days: Option<Vec<f64>>,
    pub raw_strings: Option<Vec<String>>,
    pub raw_ms: Option<Vec<i64>>,
    pub raw_ms_strings: Option<Vec<String>>,
    pub ms_since_epoch: Option<Vec<i64>>,
    pub timestamps: Option<Vec<CivilTimestamp>>,
}
impl TimeAxis {
    /// Select rows with the same slice semantics as result reads (stored row indices).
    pub fn select(&self, rows: Slice) -> Result<TimeSelection> {
        if rows.step == 0 || rows.start > rows.end || rows.end > self.rows {
            return Err(Error::new(
                ErrorKind::InvalidRequest,
                "invalid time axis selection",
            ));
        }
        let mut idx = Vec::new();
        let mut at = rows.start;
        while at < rows.end {
            idx.push(at);
            at = match at.checked_add(rows.step) {
                Some(n) => n,
                None => break,
            };
        }
        // The public fields can be edited by callers: every present vector must still match
        // `rows` before any index is used, so a select can return an error but never panic.
        let lens = [
            self.days.as_ref().map(Vec::len),
            self.raw_strings.as_ref().map(Vec::len),
            self.raw_ms.as_ref().map(Vec::len),
            self.raw_ms_strings.as_ref().map(Vec::len),
            self.ms_since_epoch.as_ref().map(Vec::len),
            self.timestamps.as_ref().map(Vec::len),
        ];
        if lens.iter().flatten().any(|&l| l as u64 != self.rows) {
            return Err(corrupt(format!(
                "time axis vectors disagree with rows ({}): {lens:?}",
                self.rows
            )));
        }
        fn pick<T: Clone>(v: &Option<Vec<T>>, idx: &[u64]) -> Option<Vec<T>> {
            v.as_ref()
                .map(|v| idx.iter().map(|&i| v[i as usize].clone()).collect())
        }
        Ok(TimeSelection {
            days: pick(&self.days, &idx),
            raw_strings: pick(&self.raw_strings, &idx),
            raw_ms: pick(&self.raw_ms, &idx),
            raw_ms_strings: pick(&self.raw_ms_strings, &idx),
            ms_since_epoch: pick(&self.ms_since_epoch, &idx),
            timestamps: pick(&self.timestamps, &idx),
            row_indices: idx,
        })
    }
}

impl ReadOnlyHdfSession {
    fn time_rank1(&self, path: &str, max_bytes: usize) -> Result<crate::hdf5_reader::Dataset> {
        let ds = self.file.dataset(path).map_err(map_error)?;
        if ds.shape().len() != 1 {
            return Err(corrupt(format!("{path} must be rank one")));
        }
        if ds.shape()[0] == 0 {
            return Err(unsupported(
                UnsupportedCode::Schema,
                format!("{path} is an empty time axis"),
            ));
        }
        if ds.raw_byte_len().map_err(map_error)? > max_bytes {
            return Err(Error::new(
                ErrorKind::InvalidRequest,
                format!("{path} exceeds caller byte limit"),
            )
            .with_code(UnsupportedCode::ResourceLimit));
        }
        Ok(ds)
    }
    /// Read the time axes found directly in `group` (`Time`, `Time Date Stamp`,
    /// `Time Date Stamp (ms)`). `expected_rows` is the result row count the axis must match.
    /// Missing every axis is NotFound; unparseable text/dtypes are Unsupported; length
    /// mismatch, non-monotonic or ms/string disagreement is Corrupt. `max_bytes` bounds each
    /// source dataset's raw bytes.
    pub fn time_axis(
        &self,
        group: &str,
        expected_rows: Option<u64>,
        max_bytes: usize,
    ) -> Result<TimeAxis> {
        self.memo(
            format!("time_axis:{:?}", (group, expected_rows, max_bytes,)),
            || self.time_axis_uncached(group, expected_rows, max_bytes),
        )
    }
    fn time_axis_uncached(
        &self,
        group: &str,
        expected_rows: Option<u64>,
        max_bytes: usize,
    ) -> Result<TimeAxis> {
        let members = self.group_members(group)?;
        let has = |n: &str| -> Result<bool> {
            if members.groups.iter().any(|g| g == n) {
                return Err(corrupt(format!("{group}/{n} is a group, expected dataset")));
            }
            Ok(members.datasets.iter().any(|d| d == n))
        };
        let (has_days, has_stamp, has_ms) = (has(DAYS)?, has(STAMP)?, has(STAMP_MS)?);
        if !(has_days || has_stamp || has_ms) {
            return Err(Error::new(
                ErrorKind::NotFound,
                format!("no time axis datasets in {group}"),
            )
            .with_subject(group));
        }
        let path = |n: &str| format!("{group}/{n}");
        let days = if has_days {
            let p = path(DAYS);
            self.time_rank1(&p, max_bytes)?;
            let cap = self.numeric_capability(&p)?;
            let v = match (cap.kind, self.read_numeric(&p, None)?.values) {
                (Some(NumericKind::F32), NumericValues::F32(v)) => {
                    v.into_iter().map(f64::from).collect::<Vec<_>>()
                }
                (Some(NumericKind::F64), NumericValues::F64(v)) => v,
                _ => {
                    return Err(unsupported(
                        UnsupportedCode::Encoding,
                        format!("{p} must be proven f32/f64"),
                    ))
                }
            };
            if v.iter().any(|x| !x.is_finite()) {
                return Err(corrupt(format!("{p} contains non-finite days")));
            }
            Some(v)
        } else {
            None
        };
        let raw_strings = if has_stamp {
            let p = path(STAMP);
            let ds = self.time_rank1(&p, max_bytes)?;
            let pr = proof::proof(&ds.raw_datatype().map_err(map_error)?)?;
            if matches!(pr.kind, proof::Kind::Compound(_)) {
                return Err(unsupported(
                    UnsupportedCode::Schema,
                    format!("{p} is compound"),
                ));
            }
            match proof::decode(&pr, &ds.read_raw_bytes().map_err(map_error)?, ds.shape()[0])? {
                MetadataValue::Strings(v) => Some(v),
                MetadataValue::Unsupported { code, reason } => {
                    return Err(unsupported(code, format!("{p}: {reason}")))
                }
                MetadataValue::Corrupt { reason } => return Err(corrupt(format!("{p}: {reason}"))),
                _ => {
                    return Err(unsupported(
                        UnsupportedCode::Encoding,
                        format!("{p} is not proven fixed text"),
                    ))
                }
            }
        } else {
            None
        };
        let mut raw_ms = None;
        let mut raw_ms_strings = None;
        if has_ms {
            let p = path(STAMP_MS);
            let ds = self.time_rank1(&p, max_bytes)?;
            let pr = proof::proof(&ds.raw_datatype().map_err(map_error)?)?;
            match pr.kind {
                proof::Kind::Text { .. } => {
                    let raw = ds.read_raw_bytes().map_err(map_error)?;
                    match proof::decode(&pr, &raw, ds.shape()[0])? {
                        MetadataValue::Strings(v) => raw_ms_strings = Some(v),
                        MetadataValue::Unsupported { code, reason } => {
                            return Err(unsupported(code, format!("{p}: {reason}")))
                        }
                        MetadataValue::Corrupt { reason } => {
                            return Err(corrupt(format!("{p}: {reason}")))
                        }
                        _ => {
                            return Err(unsupported(
                                UnsupportedCode::Encoding,
                                format!("{p} is not proven fixed text"),
                            ))
                        }
                    }
                }
                proof::Kind::Integer { .. } => {
                    let cap = self.numeric_capability(&p)?;
                    raw_ms = match (cap.kind, self.read_numeric(&p, None)?.values) {
                        (Some(NumericKind::I64), NumericValues::I64(v)) => Some(v),
                        (Some(NumericKind::U64), NumericValues::U64(v)) => Some(
                            v.into_iter()
                                .map(|x| {
                                    i64::try_from(x)
                                        .map_err(|_| corrupt(format!("{p} exceeds i64")))
                                })
                                .collect::<Result<Vec<_>>>()?,
                        ),
                        _ => {
                            return Err(unsupported(
                                UnsupportedCode::Encoding,
                                format!("{p} integer must be i64/u64"),
                            ))
                        }
                    };
                }
                _ => {
                    return Err(unsupported(
                        UnsupportedCode::Schema,
                        format!("{p} must be fixed text or an integer dataset"),
                    ))
                }
            }
        }
        let lens = [
            days.as_ref().map(Vec::len),
            raw_strings.as_ref().map(Vec::len),
            raw_ms.as_ref().map(Vec::len),
            raw_ms_strings.as_ref().map(Vec::len),
        ];
        let rows = lens.iter().flatten().next().copied().unwrap_or(0) as u64;
        if lens.iter().flatten().any(|&l| l as u64 != rows) {
            return Err(corrupt(format!(
                "time axis lengths disagree (Time/Date Stamp/ms): {lens:?}"
            )));
        }
        if let Some(e) = expected_rows {
            if e != rows {
                return Err(corrupt(format!(
                    "time axis has {rows} rows but results have {e}"
                )));
            }
        }
        let mut hour_2400_rows = Vec::new();
        let mut parse_axis =
            |texts: &Option<Vec<String>>, ms_form: bool| -> Result<Option<Vec<i64>>> {
                let Some(texts) = texts else { return Ok(None) };
                let mut out = Vec::with_capacity(texts.len());
                for (i, t) in texts.iter().enumerate() {
                    let parsed = parse_stamp(t, ms_form).map_err(|mut e| {
                        e.message = format!("row {i}: {}", e.message);
                        e
                    })?;
                    if parsed.hour_2400 {
                        hour_2400_rows.push(i as u64);
                    }
                    out.push(parsed.timestamp.ms_since_epoch());
                }
                Ok(Some(out))
            };
        let string_ms = parse_axis(&raw_strings, false)?;
        let ms_string_ms = parse_axis(&raw_ms_strings, true)?;
        hour_2400_rows.sort_unstable();
        hour_2400_rows.dedup();
        // An integer-typed ms dataset has no proven epoch: it is retained and monotonicity
        // checked, but never turned into civil time (see `integer_ms`).
        let integer_ms = raw_ms.as_ref().map(|_| {
            crate::Capability::unsupported(
                crate::UnsupportedCode::NotImplemented,
                "integer Time Date Stamp (ms) epoch is unproven; raw values retained, no civil time derived"
                    ,
            )
        });
        if let Some(raw) = &raw_ms {
            if let Some(i) = raw.windows(2).position(|w| w[1] <= w[0]) {
                return Err(corrupt(format!(
                    "integer Time Date Stamp (ms) is not strictly increasing at row {}",
                    i + 1
                )));
            }
        }
        let stored_ms = ms_string_ms;
        if let (Some(ms), Some(sm)) = (&stored_ms, &string_ms) {
            // Time Date Stamp has whole-second resolution: compare at second resolution
            // (floor), so sub-second parts of the ms axis are not a disagreement.
            if let Some(i) = ms
                .iter()
                .zip(sm)
                .position(|(a, b)| a.div_euclid(1000) != b.div_euclid(1000))
            {
                return Err(corrupt(format!(
                    "Time Date Stamp (ms) disagrees with Time Date Stamp at row {i}: {} vs {}",
                    ms[i], sm[i]
                )));
            }
        }
        let (ms_since_epoch, ms_source) = match (stored_ms, string_ms) {
            (Some(ms), _) => (Some(ms), Some(MillisecondSource::StoredStringDataset)),
            (None, Some(sm)) => (Some(sm), Some(MillisecondSource::DerivedFromStrings)),
            (None, None) => (None, None),
        };
        if let (Some(ms), Some(d)) = (&ms_since_epoch, &days) {
            // Origin-independent: elapsed days from row 0 against elapsed civil ms. The
            // tolerance is 1.5 ms plus 4 f32 epsilons of the largest stored day value for an
            // f32 axis (every value exactly representable in f32), else 1e-12 relative.
            let f32_axis = d.iter().all(|x| f64::from(*x as f32) == *x);
            let max_abs = d.iter().fold(0.0f64, |a, x| a.max(x.abs()));
            let rel = if f32_axis {
                4.0 * f64::from(f32::EPSILON)
            } else {
                1e-12
            };
            let tol = 1.5 + max_abs * rel * 86_400_000.0;
            for i in 0..d.len() {
                let by_days = (d[i] - d[0]) * 86_400_000.0;
                let elapsed = i128::from(ms[i]) - i128::from(ms[0]);
                let by_ms = elapsed as f64;
                if (by_days - by_ms).abs() > tol {
                    return Err(corrupt(format!(
                        "Time (days) disagrees with the stamps at row {i}: {by_days} ms vs {by_ms} ms elapsed"
                    )));
                }
            }
        }
        if let Some(ms) = &ms_since_epoch {
            if let Some(i) = ms.windows(2).position(|w| w[1] <= w[0]) {
                return Err(corrupt(format!(
                    "time axis is not strictly increasing at row {}",
                    i + 1
                )));
            }
        }
        if let Some(d) = &days {
            if let Some(i) = d.windows(2).position(|w| w[1] <= w[0]) {
                return Err(corrupt(format!(
                    "Time days are not strictly increasing at row {}",
                    i + 1
                )));
            }
        }
        let timestamps = ms_since_epoch
            .as_ref()
            .map(|ms| {
                ms.iter()
                    .map(|&m| CivilTimestamp::from_ms_since_epoch(m))
                    .collect::<Result<Vec<_>>>()
            })
            .transpose()?;
        Ok(TimeAxis {
            group: group.into(),
            rows,
            days,
            raw_strings,
            raw_ms,
            raw_ms_strings,
            integer_ms,
            hour_2400_rows,
            epoch: TimeEpoch::CivilUnix1970,
            ms_source,
            ms_since_epoch,
            timestamps,
        })
    }
    /// Stored time-row counts of the known unsteady result datasets that exist
    /// (1D XS Water Surface/Flow and each 2D area Water Surface). Missing ones are skipped.
    fn unsteady_result_rows(&self) -> Result<Vec<(String, u64)>> {
        let mut paths = vec![
            format!("{UNSTEADY}/Cross Sections/Water Surface"),
            format!("{UNSTEADY}/Cross Sections/Flow"),
        ];
        let areas = format!("{UNSTEADY}/2D Flow Areas");
        if let Some(m) = absent_ok(self.group_members(&areas))? {
            paths.extend(
                m.groups
                    .iter()
                    .map(|a| format!("{areas}/{a}/Water Surface")),
            );
        }
        let mut out = Vec::new();
        for p in paths {
            if let Some(ds) = absent_ok(self.file.dataset(&p).map_err(map_error))? {
                if ds.shape().is_empty() {
                    return Err(corrupt(format!("{p} is scalar")));
                }
                let rows = ds.shape()[0];
                out.push((p, rows));
            }
        }
        Ok(out)
    }
    /// Base unsteady time axis, validated against every existing known result dataset's row
    /// count (those must also agree with each other). With no result dataset, only the
    /// axes' own consistency is checked.
    pub fn unsteady_time_axis(&self, max_bytes: usize) -> Result<TimeAxis> {
        self.memo(format!("unsteady_time_axis:{:?}", (max_bytes,)), || {
            let rows = self.unsteady_result_rows()?;
            let expected = rows.first().map(|r| r.1);
            if let Some((p, n)) = rows.iter().find(|r| Some(r.1) != expected) {
                return Err(corrupt(format!(
                    "result row counts disagree: {p} has {n}, {} has {}",
                    rows[0].0, rows[0].1
                )));
            }
            self.time_axis(UNSTEADY_TIME_GROUP, expected, max_bytes)
        })
    }
    /// Timestamps for stored result rows; numeric result reads are unchanged.
    pub fn unsteady_row_times(&self, rows: Slice, max_bytes: usize) -> Result<TimeSelection> {
        self.unsteady_time_axis(max_bytes)?.select(rows)
    }
}
