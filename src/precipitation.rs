//! Gridded precipitation stored in HEC-RAS plan and unsteady-flow HDF files, as rasters.
//!
//! [`PrecipitationGrid::open`] reads the grid definition (`Raster Rows`, `Raster Cols`,
//! `Raster Cellsize`, `Raster Left`, `Raster Top`, `Projection`, `Units`, `Data Type`,
//! `NoData`) and the timestamps of `Event Conditions/Meteorology/Precipitation`, from the
//! processed values (`Values`, written by a completed plan) or the imported source grids
//! (`Imported Raster Data/Values`). [`PrecipitationSteps`] then yields per-timestep incremental
//! and cumulative grids, reading a chunk row at a time, so memory follows the grid and not the
//! number of timesteps.
//!
//! The rules are those of ras2cng's `precipitation.py` (h5py + NumPy), so a writer that
//! changes only the I/O produces identical files: values are read as float32 and worked in
//! float64; cells within `np.isclose` of `NoData` are missing; processed values are timestep
//! amounts and cumulative grids are their running sum (missing cells count as zero, cells
//! never valid stay missing); cumulative sources (`Data Type` starting with `cumul`) are
//! differenced instead; unit conversion multiplies the float32 result in float64. Attribute
//! values are decoded as that module decodes them (a float32 attribute through its shortest
//! decimal text).
use crate::hdf::metadata::{MetadataValue, TypedAttribute};
use crate::hdf::types::AttributeType;
use crate::hdf::{NumericValues, ReadOnlyHdfSession, Slice};
use crate::land_cover::{has_dataset, has_group};
use crate::{Error, ErrorKind, Result, UnsupportedCode};

pub const GROUP: &str = "Event Conditions/Meteorology/Precipitation";
pub const PROCESSED_VALUES: &str = "Event Conditions/Meteorology/Precipitation/Values";
pub const IMPORTED_VALUES: &str =
    "Event Conditions/Meteorology/Precipitation/Imported Raster Data/Values";
pub const TIMESTAMP_DATASET: &str = "Event Conditions/Meteorology/Precipitation/Timestamp";
pub const MM_PER_INCH: f64 = 25.4;

/// Which stored grids to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Processed values when present, else imported.
    Auto,
    Processed,
    Imported,
}
impl Source {
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "auto" => Ok(Self::Auto),
            "processed" => Ok(Self::Processed),
            "imported" => Ok(Self::Imported),
            _ => Err(Error::new(
                ErrorKind::InvalidRequest,
                format!("Invalid precipitation source: {s}"),
            )),
        }
    }
}

/// The grid definition and timestamps (ras2cng `PrecipitationGridInfo`).
#[derive(Debug, Clone, PartialEq)]
pub struct PrecipitationGrid {
    /// `processed` or `imported`.
    pub source: String,
    pub values_path: String,
    pub timestamps: Vec<String>,
    pub rows: usize,
    pub cols: usize,
    pub cellsize: f64,
    pub left: f64,
    pub top: f64,
    pub projection: Option<String>,
    pub units: Option<String>,
    pub data_type: Option<String>,
    pub nodata: Option<f64>,
    pub source_is_cumulative: bool,
}

/// One timestep: `rows x cols` values, row-major. Missing cells are NaN, or `NoData` (float32)
/// when the grid has one, exactly as ras2cng writes them.
#[derive(Debug, Clone, PartialEq)]
pub struct PrecipitationStep {
    pub index: usize,
    pub incremental: Vec<f32>,
    pub cumulative: Vec<f32>,
}

/// `str(value)` of the h5py attribute value, then `_decode`'s strip.
fn decoded(a: &TypedAttribute) -> Result<String> {
    let text = match (&a.value, &a.attribute_type) {
        (MetadataValue::Strings(v), _) => {
            // Scalar bytes -> text; arrays -> " ".join of decoded elements.
            v.iter()
                .map(|x| x.trim().to_string())
                .collect::<Vec<_>>()
                .join(" ")
        }
        // A scalar NumPy float32 prints its own shortest digits; array elements go through
        // `tolist()` (Python floats), which widens them first.
        (MetadataValue::Float(v), AttributeType::Float { bytes, .. }) => v
            .iter()
            .map(|x| py_float_repr(*x, *bytes == 4 && a.shape.is_empty()))
            .collect::<Vec<_>>()
            .join(" "),
        (MetadataValue::Signed(v), _) => v
            .iter()
            .map(|x| x.to_string())
            .collect::<Vec<_>>()
            .join(" "),
        (MetadataValue::Unsigned(v), _) => v
            .iter()
            .map(|x| x.to_string())
            .collect::<Vec<_>>()
            .join(" "),
        _ => {
            return Err(Error::unsupported(
                UnsupportedCode::Encoding,
                format!(
                    "precipitation attribute {:?}: unsupported shape or type",
                    a.name
                ),
            ))
        }
    };
    Ok(text.trim().to_string())
}

/// NumPy's `str()` of a float scalar: shortest round-trip digits (`repr`).
fn py_float_repr(v: f64, single: bool) -> String {
    if v.is_nan() {
        return "nan".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "inf".into() } else { "-inf".into() };
    }
    if single {
        format!("{:?}", v as f32)
    } else {
        format!("{v:?}")
    }
}

fn parse_float(name: &str, text: &str) -> Result<f64> {
    let t = text.trim();
    let lower = t.to_ascii_lowercase();
    let v = match lower.as_str() {
        "nan" | "+nan" | "-nan" => f64::NAN,
        "inf" | "+inf" | "infinity" | "+infinity" => f64::INFINITY,
        "-inf" | "-infinity" => f64::NEG_INFINITY,
        _ => t.parse::<f64>().map_err(|_| {
            Error::new(
                ErrorKind::InvalidRequest,
                format!("precipitation attribute {name}: not a number: {t:?}"),
            )
        })?,
    };
    Ok(v)
}

impl PrecipitationGrid {
    /// `read_precipitation_grid_info`.
    pub fn open(s: &ReadOnlyHdfSession, source: Source) -> Result<Self> {
        if !has_group(s, GROUP)? {
            return Err(Error::new(
                ErrorKind::NotFound,
                format!("No gridded precipitation group found: {GROUP}"),
            ));
        }
        let processed = has_dataset(s, PROCESSED_VALUES)?;
        let imported = has_dataset(s, IMPORTED_VALUES)?;
        let (values_path, label) = match source {
            Source::Processed if !processed => {
                return Err(Error::new(
                    ErrorKind::NotFound,
                    format!("Processed precipitation values not found: {PROCESSED_VALUES}"),
                ))
            }
            Source::Imported if !imported => {
                return Err(Error::new(
                    ErrorKind::NotFound,
                    format!("Imported precipitation raster values not found: {IMPORTED_VALUES}"),
                ))
            }
            Source::Processed => (PROCESSED_VALUES, "processed"),
            Source::Imported => (IMPORTED_VALUES, "imported"),
            Source::Auto if processed => (PROCESSED_VALUES, "processed"),
            Source::Auto if imported => (IMPORTED_VALUES, "imported"),
            Source::Auto => {
                return Err(Error::new(
                    ErrorKind::NotFound,
                    "No gridded precipitation values found",
                ))
            }
        };
        // Group attributes, then the dataset's (the dataset wins).
        let mut attrs: Vec<TypedAttribute> = s.typed_group_attributes(GROUP)?;
        let ds_attrs = s
            .typed_dataset_attributes(values_path, 16 * 1024 * 1024)?
            .into_value()
            .unwrap_or_default();
        for a in ds_attrs {
            if let Some(e) = attrs.iter_mut().find(|e| e.name == a.name) {
                *e = a;
            } else {
                attrs.push(a);
            }
        }
        let get = |name: &str| -> Result<Option<String>> {
            match attrs.iter().find(|a| a.name == name) {
                None => Ok(None),
                Some(a) => {
                    let t = decoded(a)?;
                    Ok((!t.is_empty()).then_some(t))
                }
            }
        };
        let req_f = |name: &str| -> Result<f64> {
            let t = get(name)?.ok_or_else(|| {
                Error::new(
                    ErrorKind::NotFound,
                    format!("Missing precipitation raster attribute: {name}"),
                )
            })?;
            parse_float(name, &t)
        };
        let req_i = |name: &str| -> Result<usize> {
            let v = req_f(name)?.trunc();
            if !(v.is_finite() && v >= 0.0 && v <= u32::MAX as f64) {
                return Err(Error::new(
                    ErrorKind::InvalidRequest,
                    format!("precipitation attribute {name} out of range"),
                ));
            }
            Ok(v as usize)
        };
        let rows = req_i("Raster Rows")?;
        let cols = req_i("Raster Cols")?;
        let cap = s.numeric_capability(values_path)?;
        let shape = cap.shape.clone();
        let expected = rows
            .checked_mul(cols)
            .ok_or_else(|| Error::new(ErrorKind::InvalidRequest, "grid size overflow"))?;
        if shape.len() < 2 || shape[shape.len() - 1] as usize != expected {
            return Err(Error::new(
                ErrorKind::InvalidRequest,
                format!(
                    "Precipitation values at {values_path:?} have shape {shape:?}; expected the final dimension to equal Raster Rows x Raster Cols ({expected})."
                ),
            ));
        }
        if shape.len() != 2 {
            return Err(Error::unsupported(
                UnsupportedCode::Schema,
                format!("Unsupported precipitation values shape: {shape:?}"),
            ));
        }
        let steps = shape[0] as usize;
        let timestamps = read_timestamps(s, &attrs, steps)?;
        let data_type = get("Data Type")?;
        let cumulative = match data_type.as_deref().map(|d| d.trim().to_lowercase()) {
            None => false,
            Some(d) if d.is_empty() || d == "per-cum" => false,
            Some(d) => d.starts_with("cumul"),
        };
        Ok(Self {
            source: label.into(),
            values_path: values_path.into(),
            timestamps,
            rows,
            cols,
            cellsize: req_f("Raster Cellsize")?,
            left: req_f("Raster Left")?,
            top: req_f("Raster Top")?,
            projection: get("Projection")?,
            units: get("Units")?,
            data_type,
            nodata: match get("NoData")? {
                Some(t) => Some(parse_float("NoData", &t)?),
                None => None,
            },
            source_is_cumulative: cumulative,
        })
    }
}

/// Streams [`PrecipitationStep`]s in time order; see [`PrecipitationSteps::new`].
#[derive(Debug)]
pub struct PrecipitationSteps {
    session: ReadOnlyHdfSession,
    grid: PrecipitationGrid,
    indices: Vec<usize>,
    wanted: Vec<bool>,
    end: usize,
    factor: f64,
    units: Option<String>,
    max_bytes: usize,
    batch: usize,
    buffer: Vec<f32>,
    buffer_start: usize,
    next: usize,
    running: Vec<f64>,
    previous: Vec<f64>,
    ever_valid: Vec<bool>,
}

impl PrecipitationSteps {
    /// Prepare `indices` (every timestep when `None`; duplicates dropped) in `units`
    /// (`native`, `in` or `mm`). A timestep over `max_bytes` is refused. Steps come in time
    /// order whatever the order of `indices`, because cumulative grids build on all earlier
    /// steps; [`indices`](Self::indices) keeps the requested order.
    pub fn new(
        session: ReadOnlyHdfSession,
        grid: PrecipitationGrid,
        indices: Option<&[usize]>,
        units: &str,
        max_bytes: usize,
    ) -> Result<Self> {
        let (factor, target) = unit_conversion(grid.units.as_deref(), units)?;
        let steps = grid.timestamps.len();
        let indices: Vec<usize> = match indices {
            None => (0..steps).collect(),
            Some(v) => {
                let mut out: Vec<usize> = Vec::new();
                for i in v {
                    if *i >= steps {
                        return Err(Error::new(
                            ErrorKind::InvalidRequest,
                            format!("Precipitation timestep index out of range: {i}"),
                        ));
                    }
                    if !out.contains(i) {
                        out.push(*i);
                    }
                }
                out
            }
        };
        let mut wanted = vec![false; steps];
        for i in &indices {
            wanted[*i] = true;
        }
        let end = indices.iter().max().map_or(0, |i| i + 1);
        let n = grid.rows * grid.cols;
        let step_bytes = n.saturating_mul(std::mem::size_of::<f32>());
        // Read whole chunk rows, so a chunk holding several timesteps is decoded once.
        let chunk_rows = session
            .numeric_capability(&grid.values_path)?
            .chunks
            .and_then(|c| c.first().copied())
            .map_or(1, |c| c.max(1) as usize);
        let batch = if chunk_rows.saturating_mul(step_bytes) <= max_bytes {
            chunk_rows
        } else {
            1
        };
        let mut this = Self {
            session,
            grid,
            indices,
            wanted,
            end,
            factor,
            units: target,
            max_bytes,
            batch,
            buffer: Vec::new(),
            buffer_start: 0,
            next: 0,
            running: vec![0.0; n],
            previous: vec![0.0; n],
            ever_valid: Vec::new(),
        };
        if !this.grid.source_is_cumulative {
            this.ever_valid = this.scan_ever_valid()?;
        }
        Ok(this)
    }

    pub fn grid(&self) -> &PrecipitationGrid {
        &self.grid
    }
    /// The selected timesteps, in the requested order.
    pub fn indices(&self) -> &[usize] {
        &self.indices
    }
    /// The units tag of the conversion (`None`: unchanged from the grid's).
    pub fn units(&self) -> Option<&str> {
        self.units.as_deref()
    }

    fn read_batch(&self, start: usize) -> Result<Vec<f32>> {
        let n = self.grid.rows * self.grid.cols;
        let stop = (start + self.batch).min(self.grid.timestamps.len());
        let raw = self.session.read_numeric_bounded(
            &self.grid.values_path,
            Some(&[
                Slice::new(start as u64, stop as u64, 1),
                Slice::new(0, n as u64, 1),
            ]),
            self.max_bytes,
        )?;
        match raw.values {
            NumericValues::F32(v) => Ok(v),
            NumericValues::F64(v) => Ok(v.into_iter().map(|x| x as f32).collect()),
            _ => Err(Error::unsupported(
                UnsupportedCode::Schema,
                "precipitation values are not floating point",
            )),
        }
    }

    /// Cells with a value at any timestep of the whole dataset (ras2cng's `static_nodata`,
    /// inverted). Stops at the first batch where every cell has been seen.
    fn scan_ever_valid(&self) -> Result<Vec<bool>> {
        let n = self.grid.rows * self.grid.cols;
        let mut seen = vec![false; n];
        let mut left = n;
        let steps = self.grid.timestamps.len();
        let mut start = 0;
        while start < steps && left > 0 {
            for row in self.read_batch(start)?.chunks_exact(n.max(1)) {
                for (p, v) in row.iter().enumerate() {
                    if !seen[p] && !missing(*v, self.grid.nodata).is_nan() {
                        seen[p] = true;
                        left -= 1;
                    }
                }
            }
            start += self.batch;
        }
        Ok(seen)
    }

    /// The next selected timestep, or `None` after the last.
    pub fn next_step(&mut self) -> Result<Option<PrecipitationStep>> {
        let n = self.grid.rows * self.grid.cols;
        while self.next < self.end {
            let t = self.next;
            if self.buffer.is_empty() || t >= self.buffer_start + self.batch {
                self.buffer = self.read_batch(t)?;
                self.buffer_start = t;
            }
            let at = (t - self.buffer_start) * n;
            let row = &self.buffer[at..at + n];
            let want = self.wanted[t];
            let mut inc = vec![0f32; if want { n } else { 0 }];
            let mut cum = vec![0f32; if want { n } else { 0 }];
            let nodata = self.grid.nodata;
            for p in 0..n {
                let w = missing(row[p], nodata);
                let (i, c) = if self.grid.source_is_cumulative {
                    let i = w - self.previous[p];
                    self.previous[p] = w;
                    (i as f32, w as f32)
                } else {
                    if !w.is_nan() {
                        self.running[p] += w;
                    }
                    let c = if self.ever_valid[p] {
                        self.running[p] as f32
                    } else {
                        f32::NAN
                    };
                    (w as f32, c)
                };
                if want {
                    inc[p] = i;
                    cum[p] = c;
                }
            }
            self.next += 1;
            if want {
                finish(&mut inc, self.factor, nodata);
                finish(&mut cum, self.factor, nodata);
                return Ok(Some(PrecipitationStep {
                    index: t,
                    incremental: inc,
                    cumulative: cum,
                }));
            }
        }
        Ok(None)
    }
}

/// The value in float64, NaN where `np.isclose(value, nodata)`: `|a - b| <= 1e-8 + 1e-5 * |b|`,
/// and equal infinities are close.
fn missing(value: f32, nodata: Option<f64>) -> f64 {
    let w = f64::from(value);
    match nodata {
        Some(b)
            if w == b
                || (w.is_finite() && b.is_finite() && (w - b).abs() <= 1e-8 + 1e-5 * b.abs()) =>
        {
            f64::NAN
        }
        _ => w,
    }
}

/// Unit conversion in float64, then NaN back to the float32 `NoData`.
fn finish(values: &mut [f32], factor: f64, nodata: Option<f64>) {
    for x in values.iter_mut() {
        if factor != 1.0 {
            *x = (f64::from(*x) * factor) as f32;
        }
        if let Some(nd) = nodata {
            if x.is_nan() {
                *x = nd as f32;
            }
        }
    }
}

fn read_timestamps(
    s: &ReadOnlyHdfSession,
    attrs: &[TypedAttribute],
    count: usize,
) -> Result<Vec<String>> {
    let mut ts: Vec<String> = if has_dataset(s, TIMESTAMP_DATASET)? {
        s.read_strings(TIMESTAMP_DATASET)?
            .into_iter()
            .map(|v| v.trim().to_string())
            .collect()
    } else {
        match attrs.iter().find(|a| a.name == "Times") {
            None => Vec::new(),
            Some(a) => match &a.value {
                MetadataValue::Strings(v) if !a.shape.is_empty() => {
                    v.iter().map(|x| x.trim().to_string()).collect()
                }
                MetadataValue::Strings(v) => parse_times_text(v.join(" ").trim()),
                _ => {
                    return Err(Error::unsupported(
                        UnsupportedCode::Encoding,
                        "precipitation Times attribute is not text",
                    ))
                }
            },
        }
    };
    if ts.is_empty() {
        ts = (0..count).map(|i| i.to_string()).collect();
    }
    if ts.len() != count {
        return Err(Error::new(
            ErrorKind::InvalidRequest,
            format!(
                "Precipitation timestamp count ({}) does not match value timestep count ({count}).",
                ts.len()
            ),
        ));
    }
    Ok(ts)
}

/// `_parse_times_attr` for a text value: quoted items (`b'...'` or `'...'`), else split on
/// commas, semicolons and newlines.
fn parse_times_text(text: &str) -> Vec<String> {
    let mut quoted = Vec::new();
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\'' {
            if let Some(end) = text[i + 1..].find('\'') {
                let item = &text[i + 1..i + 1 + end];
                if !item.is_empty() {
                    quoted.push(item.to_string());
                    i += end + 2;
                    continue;
                }
            }
        }
        i += 1;
    }
    if !quoted.is_empty() {
        return quoted;
    }
    text.split([',', ';', '\n'])
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(String::from)
        .collect()
}

fn normalize_unit(u: Option<&str>) -> Option<String> {
    let t = u?.trim().to_lowercase();
    if t.is_empty() {
        return None;
    }
    Some(match t.as_str() {
        "in" | "inch" | "inches" | "\"" => "in".into(),
        "mm" | "millimeter" | "millimeters" | "millimetre" | "millimetres" => "mm".into(),
        _ => t,
    })
}

/// `_unit_conversion`: `(factor, units tag)`.
pub fn unit_conversion(source: Option<&str>, target: &str) -> Result<(f64, Option<String>)> {
    if target == "native" {
        return Ok((1.0, None));
    }
    if target != "in" && target != "mm" {
        return Err(Error::new(
            ErrorKind::InvalidRequest,
            format!("Invalid precipitation units: {target}"),
        ));
    }
    let src = normalize_unit(source);
    match src.as_deref() {
        None => Ok((1.0, Some(target.into()))),
        Some(s) if s == target => Ok((1.0, Some(target.into()))),
        Some("mm") => Ok((1.0 / MM_PER_INCH, Some("in".into()))),
        Some("in") => Ok((MM_PER_INCH, Some("mm".into()))),
        Some(_) => Err(Error::new(
            ErrorKind::InvalidRequest,
            format!(
                "Cannot convert precipitation units from {:?} to {target:?}",
                source.unwrap_or("")
            ),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn units_follow_ras2cng() {
        assert_eq!(unit_conversion(Some("in"), "native").unwrap(), (1.0, None));
        assert_eq!(
            unit_conversion(Some("in"), "mm").unwrap(),
            (25.4, Some("mm".into()))
        );
        assert_eq!(
            unit_conversion(Some("Millimeters"), "in").unwrap(),
            (1.0 / 25.4, Some("in".into()))
        );
        assert_eq!(
            unit_conversion(None, "in").unwrap(),
            (1.0, Some("in".into()))
        );
        assert_eq!(
            unit_conversion(Some("in"), "in").unwrap(),
            (1.0, Some("in".into()))
        );
        assert!(unit_conversion(Some("cm"), "in").is_err());
        assert!(unit_conversion(Some("in"), "ft").is_err());
    }

    #[test]
    fn missing_follows_np_isclose() {
        assert!(missing(-9999.0, Some(-9999.0)).is_nan());
        assert!(missing(-9999.05, Some(-9999.0)).is_nan());
        assert_eq!(missing(-9990.0, Some(-9999.0)), -9990.0);
        assert_eq!(missing(1.5, None), 1.5);
        assert!(missing(f32::INFINITY, Some(f64::INFINITY)).is_nan());
        assert!(missing(f32::NAN, Some(1.0)).is_nan());
    }

    #[test]
    fn times_text_parses_quoted_and_delimited() {
        assert_eq!(
            parse_times_text("[b'01Jan2000 00:00:00' b'01Jan2000 01:00:00']"),
            vec!["01Jan2000 00:00:00", "01Jan2000 01:00:00"]
        );
        assert_eq!(parse_times_text("a, b;c\n d"), vec!["a", "b", "c", "d"]);
    }

    #[test]
    fn float_repr_matches_numpy_str() {
        assert_eq!(py_float_repr(2000.0, false), "2000.0");
        assert_eq!(py_float_repr(0.1f32 as f64, true), "0.1");
        assert_eq!(parse_float("x", "0.1").unwrap(), 0.1);
        assert!(parse_float("x", "nan").unwrap().is_nan());
        assert_eq!(Source::parse("auto").unwrap(), Source::Auto);
        assert!(Source::parse("x").is_err());
    }
}
