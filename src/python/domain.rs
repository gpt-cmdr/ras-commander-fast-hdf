//! Explicit conversion only; engine retains schema and capability decisions.
use crate::crs::{CrsEvidence, EvidenceStatus};
use crate::hdf::metadata::MetadataValue;
use crate::hdf::xs1d::{PlanResultMode, XsVariable};
use crate::hdf::{AttributeType, ByteOrder, ReadOnlyHdfSession, ResultMode, Slice};
use crate::{Capability, UnsupportedCode};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};
use std::path::PathBuf;
macro_rules! dict { ($py:expr, $($k:expr => $v:expr),* $(,)?) => {{ let d=PyDict::new($py); $(d.set_item($k,$v)?;)* d }}; }
pub(crate) fn capability<'py>(py: Python<'py>, c: &Capability) -> PyResult<Bound<'py, PyDict>> {
    Ok(dict!(py,"status"=>c.as_str(),"code"=>c.code().map(|v|v.as_str()),"reason"=>c.reason()))
}
fn order(o: ByteOrder) -> &'static str {
    o.as_str()
}
pub(crate) fn evidence_dict<'py>(
    py: Python<'py>,
    kind: &str,
    raw: Option<&[u8]>,
    c: &Capability,
) -> PyResult<Bound<'py, PyDict>> {
    Ok(
        dict!(py,"kind"=>kind,"value"=>py.None(),"raw_bytes"=>raw.map(|b|PyBytes::new(py,b)),"capability"=>capability(py,c)?),
    )
}
pub(crate) fn identity<'py>(
    py: Python<'py>,
    id: &crate::hdf::Identity,
) -> PyResult<Bound<'py, PyDict>> {
    Ok(dict!(py,"source"=>&id.source,"table"=>&id.table,"row"=>id.row))
}
/// [`identity`] of each element. Runs of equal `source`/`table` share one converted Python
/// object (an equal, immutable `pathlib.Path`/`str`) instead of converting it per row.
pub(crate) fn identities<'py>(
    py: Python<'py>,
    ids: &[crate::hdf::Identity],
) -> PyResult<Vec<Bound<'py, PyDict>>> {
    let mut source: Option<(&std::path::Path, Bound<'py, PyAny>)> = None;
    let mut table: Option<(&str, Bound<'py, PyAny>)> = None;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let s = match &source {
            Some((path, object)) if *path == id.source.as_path() => object.clone(),
            _ => {
                let object = id.source.as_path().into_pyobject(py)?.into_any();
                source = Some((id.source.as_path(), object.clone()));
                object
            }
        };
        let t = match &table {
            Some((name, object)) if *name == id.table => object.clone(),
            _ => {
                let object = id.table.as_str().into_pyobject(py)?.into_any();
                table = Some((id.table.as_str(), object.clone()));
                object
            }
        };
        out.push(dict!(py,"source"=>s,"table"=>t,"row"=>id.row));
    }
    Ok(out)
}
const _: () = assert!(crate::API_VERSION == 10);
const _: () = assert!(crate::hdf::DEFAULT_MAX_BYTES == 67108864);
pub(crate) fn attribute_type<'py>(
    py: Python<'py>,
    t: &AttributeType,
) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    match t {
        AttributeType::Int {
            signed,
            bytes,
            order: o,
        } => {
            d.set_item("kind", "integer")?;
            d.set_item("signed", signed)?;
            d.set_item("bytes", bytes)?;
            d.set_item("byte_order", order(*o))?;
        }
        AttributeType::Float { bytes, order: o } => {
            d.set_item("kind", "float")?;
            d.set_item("bytes", bytes)?;
            d.set_item("byte_order", order(*o))?;
        }
        AttributeType::FixedString {
            charset,
            padding,
            width,
        } => {
            use crate::hdf::{StringCharset as C, StringPadding as P};
            d.set_item("kind", "fixed_string")?;
            d.set_item("width", width)?;
            d.set_item(
                "charset",
                match charset {
                    C::Ascii => "ascii",
                    C::Utf8 => "utf8",
                    _ => "unknown",
                },
            )?;
            d.set_item(
                "padding",
                match padding {
                    P::NullTerminate => "null_terminate",
                    P::NullPad => "null_pad",
                    P::SpacePad => "space_pad",
                    _ => "unknown",
                },
            )?;
        }
        AttributeType::Bitfield { size, order: o } => {
            // h5py exposes a bitfield as the unsigned integer of the same size and order.
            d.set_item("kind", "bitfield")?;
            d.set_item("bytes", size)?;
            d.set_item("byte_order", order(*o))?;
        }
        AttributeType::Compound => d.set_item("kind", "compound")?,
        AttributeType::Unsupported { .. } => {
            d.set_item("kind", "unsupported")?;
            d.set_item("capability", capability(py, &t.capability())?)?;
        }
        _ => d.set_item("kind", "unknown")?,
    }
    Ok(d)
}
fn evidence<'py>(py: Python<'py>, e: &CrsEvidence) -> PyResult<Bound<'py, PyDict>> {
    let d = evidence_dict(py, status(&e.status), Some(&e.raw_bytes), &e.capability())?;
    d.set_item("value", &e.exact_wkt)?;
    d.set_item("source", &e.source)?;
    d.set_item("exact_wkt", &e.exact_wkt)?;
    d.set_item("unit_clauses", &e.unit_clauses)?;
    d.set_item("axis_clauses", &e.axis_clauses)?;
    Ok(d)
}
fn status(s: &EvidenceStatus) -> &'static str {
    match s {
        EvidenceStatus::Unknown => "unknown",
        EvidenceStatus::Unvalidated => "unvalidated",
        EvidenceStatus::Invalid => "invalid",
        EvidenceStatus::Conflict => "conflict",
        EvidenceStatus::Unsupported { .. } => "unsupported",
        EvidenceStatus::Corrupt { .. } => "corrupt",
        _ => "unknown",
    }
}
fn mode(m: PlanResultMode) -> &'static str {
    m.as_str()
}
#[pyfunction]
fn capabilities(py: Python<'_>) -> PyResult<Bound<'_, PyDict>> {
    crate::python::panic_guard::guard(|| {
        let engine = crate::capabilities();
        let entrypoints = engine
            .entrypoints
            .iter()
            .map(|e| Ok(dict!(py,"owner"=>e.owner,"name"=>e.name,"feature"=>e.feature)))
            .collect::<PyResult<Vec<_>>>()?;
        let engine_capabilities = dict!(py,"api_version"=>engine.api_version,"features"=>engine.features,"entrypoints"=>entrypoints);
        // The process-wide in-flight decode budget (additive key).
        let decode_budget = match crate::hdf::DecodeBudget::global() {
            Ok(budget) => {
                let s = budget.stats();
                dict!(py,"max_bytes"=>s.max_bytes,"wait_ms"=>u64::try_from(s.wait.as_millis()).unwrap_or(u64::MAX),"in_flight_bytes"=>s.in_flight_bytes,"peak_in_flight_bytes"=>s.peak_in_flight_bytes,"waits"=>s.waits,"refusals"=>s.refusals)
            }
            Err(e) => dict!(py,"error"=>e.message),
        };
        Ok(
            dict!(py,"engine_api_version"=>crate::API_VERSION,"engine_capabilities"=>engine_capabilities,"default_max_bytes"=>crate::hdf::DEFAULT_MAX_BYTES,"decode_budget"=>decode_budget,"api_version"=>crate::python::API_VERSION,"engine_commit"=>env!("RAS_ENGINE_COMMIT"),"engine_features"=>env!("RAS_ENGINE_FEATURES").split(',').collect::<Vec<_>>(),"bindings"=>crate::python::REGISTERED_BINDINGS.get().ok_or_else(||pyo3::exceptions::PyRuntimeError::new_err("module registration incomplete"))?),
        )
    })
}
#[pyfunction]
fn raw_crs_evidence(
    py: Python<'_>,
    path: crate::python::session::Source,
) -> PyResult<Bound<'_, PyDict>> {
    crate::python::panic_guard::guard(|| {
        let r = path.run(py, |engine_session| engine_session.raw_crs_evidence())?;
        let ev = r
            .evidence
            .iter()
            .map(|e| evidence(py, e))
            .collect::<PyResult<Vec<_>>>()?;
        let units = r
            .hydraulic_units_evidence
            .iter()
            .map(|e| evidence(py, e))
            .collect::<PyResult<Vec<_>>>()?;
        Ok(
            dict!(py,"kind"=>status(&r.status),"capability"=>capability(py,&r.capability())?,"evidence"=>ev,"hydraulic_units"=>r.hydraulic_units,"hydraulic_units_evidence"=>units),
        )
    })
}
#[pyfunction]
#[pyo3(signature=(path,group="/"))]
fn typed_group_attributes<'py>(
    py: Python<'py>,
    path: crate::python::session::Source,
    group: &str,
) -> PyResult<Vec<Bound<'py, PyDict>>> {
    crate::python::panic_guard::guard(|| {
        let attrs = path.run(py, |engine_session| {
            engine_session.typed_group_attributes(group)
        })?;
        attrs.iter().map(|a| typed_attribute_dict(py, a)).collect()
    })
}
/// Engine API 9 `typed_dataset_attributes`: every attribute of one dataset, typed with the
/// family encoding proof. `{capability, value: [typed attribute] | None}`.
#[pyfunction]
#[pyo3(signature=(path,dataset,max_bytes=67108864))]
fn typed_dataset_attributes<'py>(
    py: Python<'py>,
    path: crate::python::session::Source,
    dataset: &str,
    max_bytes: usize,
) -> PyResult<Bound<'py, PyDict>> {
    crate::python::panic_guard::guard(|| {
        let r = path.run(py, |engine_session| {
            engine_session.typed_dataset_attributes(dataset, max_bytes)
        })?;
        let d = dict!(py,"capability"=>capability(py,r.capability())?,"value"=>py.None());
        if let Some(attrs) = r.value() {
            let values = attrs
                .iter()
                .map(|a| typed_attribute_dict(py, a))
                .collect::<PyResult<Vec<_>>>()?;
            d.set_item("value", values)?;
        }
        Ok(d)
    })
}
/// Engine API 9 `raw_fixed_strings`: a fixed-length string dataset as stored (padding kept).
/// `{capability, value: {shape, dataspace, width, padding, charset, raw_bytes} | None}`.
#[pyfunction]
#[pyo3(signature=(path,dataset,max_bytes=67108864))]
fn raw_fixed_strings<'py>(
    py: Python<'py>,
    path: crate::python::session::Source,
    dataset: &str,
    max_bytes: usize,
) -> PyResult<Bound<'py, PyDict>> {
    crate::python::panic_guard::guard(|| {
        use crate::hdf::{StringCharset as C, StringPadding as P};
        let r = path.run(py, |engine_session| {
            engine_session.raw_fixed_strings(dataset, max_bytes)
        })?;
        let d = dict!(py,"capability"=>capability(py,r.capability())?,"value"=>py.None());
        if let Some(v) = r.value() {
            let charset = match v.charset {
                C::Ascii => "ascii",
                C::Utf8 => "utf8",
                _ => "unknown",
            };
            let padding = match v.padding {
                P::NullTerminate => "null_terminate",
                P::NullPad => "null_pad",
                P::SpacePad => "space_pad",
                _ => "unknown",
            };
            d.set_item(
            "value",
            dict!(py,"shape"=>&v.shape,"dataspace"=>v.dataspace.as_str(),"width"=>v.width,"padding"=>padding,"charset"=>charset,"raw_bytes"=>PyBytes::new(py,&v.raw_bytes)),
        )?;
        }
        Ok(d)
    })
}
pub(crate) fn typed_attribute_dict<'py>(
    py: Python<'py>,
    a: &crate::hdf::metadata::TypedAttribute,
) -> PyResult<Bound<'py, PyDict>> {
    let d = dict!(py,"name"=>&a.name,"kind"=>"attribute","shape"=>&a.shape,"dataspace"=>a.dataspace.as_str(),"attribute_type"=>attribute_type(py,&a.attribute_type)?,"datatype_label"=>&a.datatype_label,"raw_bytes"=>PyBytes::new(py,&a.raw_bytes));
    let c = a.value.capability();
    match &a.value {
        MetadataValue::Signed(v) => {
            d.set_item("value", v)?;
        }
        MetadataValue::Unsigned(v) => {
            d.set_item("value", v)?;
        }
        MetadataValue::Float(v) => {
            d.set_item("value", v)?;
        }
        MetadataValue::Strings(v) => {
            d.set_item("value", v)?;
        }
        MetadataValue::Bitfield(v) => {
            d.set_item("value", PyBytes::new(py, v))?;
        }
        MetadataValue::Unsupported { .. } | MetadataValue::Corrupt { .. } => {
            d.set_item("value", py.None())?;
        }
        _ => {
            d.set_item("value", py.None())?;
        }
    };
    d.set_item("capability", capability(py, &c)?)?;
    Ok(d)
}
fn result_availability<'py>(
    py: Python<'py>,
    r: &crate::schema::ResultAvailability,
) -> PyResult<Bound<'py, PyDict>> {
    let vars=r.variables.iter().map(|v|Ok(dict!(py,"dataset"=>&v.dataset,"dataset_path"=>&v.dataset,"shape"=>&v.shape,"capability"=>capability(py,&v.capability)?))).collect::<PyResult<Vec<_>>>()?;
    Ok(dict!(py,"capability"=>capability(py,&r.capability)?,"variables"=>vars))
}
#[pyfunction]
#[pyo3(signature=(path,max_bytes=67108864))]
fn mesh_inventory(
    py: Python<'_>,
    path: crate::python::session::Source,
    max_bytes: usize,
) -> PyResult<Vec<Bound<'_, PyDict>>> {
    crate::python::panic_guard::guard(|| {
        let meshes = path.run(py, |engine_session| {
            engine_session.mesh_inventory(max_bytes)
        })?;
        meshes.into_iter().map(|m| {
        let blocks=m.output_blocks.iter().map(|b| Ok(dict!(py,"mode"=>b.mode.as_str(),"block_name"=>&b.block_name,"dataset_root"=>&b.dataset_root,"results"=>result_availability(py,&b.results)?))).collect::<PyResult<Vec<_>>>()?;
        Ok(dict!(py,"source"=>&m.id.source,"identity"=>identity(py,&m.id)?,"mesh_name"=>m.mesh_name,"physical_cells"=>m.physical_cells,"stored_cells"=>m.stored_cells,"ghost_cells"=>m.ghost_cells,"results"=>result_availability(py,&m.results)?,"output_blocks"=>blocks))
    }).collect()
    })
}
#[pyfunction]
fn plan_result_mode(
    py: Python<'_>,
    path: crate::python::session::Source,
) -> PyResult<&'static str> {
    crate::python::panic_guard::guard(|| {
        path.run(py, |engine_session| engine_session.plan_result_mode())
            .map(mode)
    })
}
#[pyfunction]
#[pyo3(signature=(path,max_bytes=67108864))]
fn steady_profile_names(
    py: Python<'_>,
    path: crate::python::session::Source,
    max_bytes: usize,
) -> PyResult<Vec<String>> {
    crate::python::panic_guard::guard(|| {
        path.run(py, |engine_session| {
            engine_session.steady_profile_names(max_bytes)
        })
    })
}
#[pyfunction]
#[pyo3(signature=(path,max_bytes=67108864))]
fn cross_sections(
    py: Python<'_>,
    path: crate::python::session::Source,
    max_bytes: usize,
) -> PyResult<Bound<'_, PyDict>> {
    crate::python::panic_guard::guard(|| {
        let g = path.run(py, |engine_session| {
            engine_session.cross_sections(max_bytes)
        })?;
        cross_section_geometry(py, g)
    })
}
pub(crate) fn cross_section_geometry(
    py: Python<'_>,
    g: crate::hdf::xs1d::CrossSectionGeometry,
) -> PyResult<Bound<'_, PyDict>> {
    let sections = g
        .sections
        .into_iter()
        .map(|s| {
            let d =
                dict!(py,"index"=>s.id.row,"identity"=>identity(py,&s.id)?,"river"=>s.river,"reach"=>s.reach,"station"=>s.station);
            macro_rules! field {
                ($name:expr,$v:expr) => {{
                    let f=evidence_dict(py,$name,$v.raw_bytes(),$v.capability())?;
                    f.set_item("value",$v.value())?;
                    d.set_item($name, f)?;
                }};
            }
            field!("name", s.name);
            field!("left_bank", s.left_bank);
            field!("right_bank", s.right_bank);
            Ok(d)
        })
        .collect::<PyResult<Vec<_>>>()?;
    Ok(
        dict!(py,"source"=>g.source,"dataset_path"=>&g.dataset,"dataset"=>g.dataset,"sections"=>sections),
    )
}
#[pyfunction]
#[pyo3(signature=(path,kind,variable,rows,sections,max_bytes=67108864,threads=None,parallel=true,*,metadata_max_bytes=67108864))]
#[allow(clippy::too_many_arguments)]
fn read_cross_section_result<'py>(
    py: Python<'py>,
    path: PathBuf,
    kind: &str,
    variable: &str,
    rows: (u64, u64, u64),
    sections: (u64, u64, u64),
    max_bytes: usize,
    threads: Option<usize>,
    parallel: bool,
    metadata_max_bytes: usize,
) -> PyResult<Bound<'py, PyAny>> {
    crate::python::panic_guard::guard(|| {
        let k = match kind {
            "steady" => ResultMode::Steady,
            "unsteady" => ResultMode::Unsteady,
            _ => return Err(PyValueError::new_err("kind must be steady or unsteady")),
        };
        let v = match variable {
            "Water Surface" => XsVariable::WaterSurface,
            "Flow" => XsVariable::Flow,
            _ => {
                return Err(PyValueError::new_err(
                    "variable must be Water Surface or Flow",
                ))
            }
        };
        let threads = crate::python::reader::threads(threads)?;
        let out = py
            .detach(|| {
                ReadOnlyHdfSession::with_options(path, threads, parallel)?
                    .read_cross_section_result(
                        k,
                        v,
                        Slice::new(rows.0, rows.1, rows.2),
                        Slice::new(sections.0, sections.1, sections.2),
                        metadata_max_bytes,
                        max_bytes,
                    )
            })
            .map_err(crate::python::reader::err)?;
        crate::python::reader::array(py, out)
    })
}
#[pyfunction]
#[pyo3(signature=(path,max_bytes=67108864))]
fn plan_capabilities(
    py: Python<'_>,
    path: crate::python::session::Source,
    max_bytes: usize,
) -> PyResult<Bound<'_, PyDict>> {
    crate::python::panic_guard::guard(|| {
        let c = path.run(py, |engine_session| {
            engine_session.plan_capabilities(max_bytes)
        })?;
        let families=c.families.iter().map(|f| {
        let family=f.family.as_str();
        Ok(dict!(py,"family"=>family,"capability"=>capability(py,&f.status)?,"datasets"=>&f.datasets))
    }).collect::<PyResult<Vec<_>>>()?;
        let structures=c.structures.iter().map(|s| Ok(dict!(py,"family"=>crate::python::structures::family_name(s.kind),"capability"=>capability(py,&s.status)?,"datasets"=>&s.datasets,"diagnostics"=>s.diagnostics.iter().map(|m|diagnostic(py,None,Some(m),None)).collect::<PyResult<Vec<_>>>()?))).collect::<PyResult<Vec<_>>>()?;
        Ok(dict!(py,"mode"=>mode(c.mode),"families"=>families,"structures"=>structures))
    })
}

fn axis_capability(a: &crate::hdf::time::TimeAxis) -> Capability {
    if a.timestamps.is_some() {
        Capability::Supported
    } else {
        a.integer_ms.clone().unwrap_or_else(|| {
            Capability::unsupported(
                UnsupportedCode::Schema,
                "numeric time axis has no proven civil epoch",
            )
        })
    }
}
#[pyfunction]
#[pyo3(signature=(path,max_bytes=67108864))]
fn unsteady_time_axis(
    py: Python<'_>,
    path: crate::python::session::Source,
    max_bytes: usize,
) -> PyResult<Bound<'_, PyDict>> {
    crate::python::panic_guard::guard(|| {
        use crate::hdf::time::{MillisecondSource, TimeEpoch};
        let a = path.run(py, |engine_session| {
            engine_session.unsteady_time_axis(max_bytes)
        })?;
        let timestamps = a
            .timestamps
            .as_ref()
            .map(|v| v.iter().map(ToString::to_string).collect::<Vec<_>>());
        let epoch = match a.epoch {
            TimeEpoch::CivilUnix1970 => "civil_unix_1970",
            _ => "unknown",
        };
        let source = a.ms_source.map(|v| match v {
            MillisecondSource::StoredStringDataset => "stored_string_dataset",
            MillisecondSource::DerivedFromStrings => "derived_from_strings",
            _ => "unknown",
        });
        Ok(
            dict!(py,"group"=>&a.group,"rows"=>a.rows,"days"=>&a.days,"raw_strings"=>&a.raw_strings,"raw_ms"=>&a.raw_ms,"raw_ms_strings"=>&a.raw_ms_strings,"ms_since_epoch"=>&a.ms_since_epoch,"timestamps"=>timestamps,"hour_2400_rows"=>&a.hour_2400_rows,"epoch"=>epoch,"ms_source"=>source,"timezone"=>py.None(),"capability"=>capability(py,&axis_capability(&a))?,"integer_ms"=>a.integer_ms.as_ref().map(|c|capability(py,c)).transpose()?),
        )
    })
}
#[pyfunction]
#[pyo3(signature=(path,rows,max_bytes=67108864))]
fn unsteady_row_times(
    py: Python<'_>,
    path: PathBuf,
    rows: (u64, u64, u64),
    max_bytes: usize,
) -> PyResult<Bound<'_, PyDict>> {
    crate::python::panic_guard::guard(|| {
        let (a, r) = py
            .detach(|| {
                let a = ReadOnlyHdfSession::open(path)?.unsteady_time_axis(max_bytes)?;
                let r = a.select(Slice::new(rows.0, rows.1, rows.2))?;
                Ok::<_, crate::Error>((a, r))
            })
            .map_err(crate::python::reader::err)?;
        let timestamps = r
            .timestamps
            .as_ref()
            .map(|v| v.iter().map(ToString::to_string).collect::<Vec<_>>());
        let hours = a
            .hour_2400_rows
            .iter()
            .copied()
            .filter(|row| r.row_indices.binary_search(row).is_ok())
            .collect::<Vec<_>>();
        Ok(
            dict!(py,"row_indices"=>&r.row_indices,"days"=>&r.days,"raw_strings"=>&r.raw_strings,"raw_ms"=>&r.raw_ms,"raw_ms_strings"=>&r.raw_ms_strings,"ms_since_epoch"=>&r.ms_since_epoch,"timestamps"=>timestamps,"hour_2400_rows"=>hours,"epoch"=>match a.epoch {crate::hdf::time::TimeEpoch::CivilUnix1970=>"civil_unix_1970",_=>"unknown"},"integer_ms"=>a.integer_ms.as_ref().map(|c|capability(py,c)).transpose()?,"capability"=>capability(py,&axis_capability(&a))?,"timezone"=>py.None()),
        )
    })
}
fn unit_evidence<'py>(
    py: Python<'py>,
    e: &crate::hdf::units::UnitsEvidence,
) -> PyResult<Bound<'py, PyDict>> {
    let d = evidence_dict(py, "units", e.raw_bytes(), e.capability())?;
    d.set_item("value", e.value())?;
    Ok(d)
}
pub(crate) fn dataset_units<'py>(
    py: Python<'py>,
    e: &crate::hdf::units::UnitsEvidence,
    source: &crate::hdf::units::UnitsSource,
    quantity: &Option<String>,
) -> PyResult<Bound<'py, PyDict>> {
    use crate::hdf::units::UnitsSource as S;
    let attribute = match source {
        S::UnitsAttribute => Some("Units"),
        S::VariableUnits => Some("Variable Units"),
        S::VariablePair => Some("Variable"),
        S::QuantityLabel { attribute } => Some(attribute.as_str()),
        _ => None,
    };
    let d = unit_evidence(py, e)?;
    d.set_item("source_attribute", attribute)?;
    d.set_item("quantity_name", quantity)?;
    Ok(d)
}
#[pyfunction]
#[pyo3(signature=(path,dataset,max_bytes=67108864))]
fn dataset_unit_evidence<'py>(
    py: Python<'py>,
    path: crate::python::session::Source,
    dataset: &str,
    max_bytes: usize,
) -> PyResult<Bound<'py, PyDict>> {
    crate::python::panic_guard::guard(|| {
        let u = path.run(py, |engine_session| {
            engine_session.dataset_units_bounded(dataset, max_bytes)
        })?;
        dataset_units(py, &u.units, &u.source, &u.quantity_name)
    })
}
#[pyfunction]
#[pyo3(signature=(path,max_bytes=67108864))]
fn result_units(
    py: Python<'_>,
    path: crate::python::session::Source,
    max_bytes: usize,
) -> PyResult<Bound<'_, PyDict>> {
    crate::python::panic_guard::guard(|| {
        let u = path.run(py, |engine_session| engine_session.result_units(max_bytes))?;
        let variables=u.variables.iter().map(|v|Ok(dict!(py,"family"=>v.family.as_str(),"area"=>&v.area,"variable"=>&v.variable,"dataset"=>&v.dataset,"dataset_path"=>&v.dataset,"units"=>dataset_units(py,&v.units,&v.source,&v.quantity_name)?))).collect::<PyResult<Vec<_>>>()?;
        let systems = u
            .unit_system
            .iter()
            .map(|e| {
                let d = unit_evidence(py, &e.value)?;
                d.set_item("kind", "unit_system")?;
                d.set_item("source", &e.source)?;
                d.set_item("source_attribute", "Units System")?;
                d.set_item("unit_system_kind", e.kind.map(|k| k.as_str()))?;
                Ok(d)
            })
            .collect::<PyResult<Vec<_>>>()?;
        Ok(dict!(py,"variables"=>variables,"unit_system"=>systems))
    })
}
#[pyfunction]
#[pyo3(signature=(path,max_bytes=67108864))]
fn mesh_name_diagnostics(
    py: Python<'_>,
    path: crate::python::session::Source,
    max_bytes: usize,
) -> PyResult<Vec<Bound<'_, PyDict>>> {
    crate::python::panic_guard::guard(|| {
        use crate::schema::MeshNameIssue as I;
        let d = path.run(py, |engine_session| {
            engine_session.mesh_name_diagnostics(max_bytes)
        })?;
        d.iter().map(|v|Ok(dict!(py,"subject"=>Some(&v.mesh_name),"message"=>None::<String>,"capability"=>None::<String>,"mesh_name"=>&v.mesh_name,"issue"=>match v.issue {I::MissingGroup=>"missing_group",I::ExtraGroup=>"extra_group",I::DuplicateAttributeName=>"duplicate_attribute_name",I::InvalidName=>"invalid_name",_=>"unknown"}))).collect()
    })
}
pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(unsteady_time_axis, m)?)?;
    m.add_function(wrap_pyfunction!(unsteady_row_times, m)?)?;
    m.add_function(wrap_pyfunction!(result_units, m)?)?;
    m.add_function(wrap_pyfunction!(dataset_unit_evidence, m)?)?;
    m.add_function(wrap_pyfunction!(mesh_name_diagnostics, m)?)?;

    m.add_function(wrap_pyfunction!(capabilities, m)?)?;
    m.add_function(wrap_pyfunction!(raw_crs_evidence, m)?)?;
    m.add_function(wrap_pyfunction!(typed_group_attributes, m)?)?;
    m.add_function(wrap_pyfunction!(typed_dataset_attributes, m)?)?;
    m.add_function(wrap_pyfunction!(raw_fixed_strings, m)?)?;
    m.add_function(wrap_pyfunction!(mesh_inventory, m)?)?;
    m.add_function(wrap_pyfunction!(cross_sections, m)?)?;
    m.add_function(wrap_pyfunction!(plan_result_mode, m)?)?;
    m.add_function(wrap_pyfunction!(steady_profile_names, m)?)?;
    m.add_function(wrap_pyfunction!(read_cross_section_result, m)?)?;
    m.add_function(wrap_pyfunction!(plan_capabilities, m)?)?;
    Ok(())
}

/// Shared envelope; None means the engine did not provide that evidence.
pub(crate) fn diagnostic<'py>(
    py: Python<'py>,
    subject: Option<&str>,
    message: Option<&str>,
    c: Option<&Capability>,
) -> PyResult<Bound<'py, PyDict>> {
    Ok(
        dict!(py,"subject"=>subject,"message"=>message,"capability"=>capability(py,c.unwrap_or(&Capability::unavailable("engine diagnostic carries no capability")))?),
    )
}

#[cfg(test)]
mod source_tests {
    use super::*;
    use crate::python::session::Source;
    use pyo3::types::PyAnyMethods;

    fn fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/hdf_numeric/numeric_types_v1.h5")
    }

    #[test]
    fn a_session_source_returns_what_a_path_source_returns() {
        pyo3::Python::initialize();
        Python::attach(|py| {
            let module = PyModule::new(py, "source_test").unwrap();
            crate::python::session::register(&module).unwrap();
            let session = module
                .getattr("HdfSession")
                .unwrap()
                .call1((fixture(),))
                .unwrap();
            let source = |o: &Bound<'_, PyAny>| o.extract::<Source>().unwrap();
            let path = fixture().into_pyobject(py).unwrap().into_any();
            for name in ["class/fixed_string", "int/u64_bounds"] {
                let by_path = raw_fixed_strings(py, source(&path), name, 67108864);
                let by_session = raw_fixed_strings(py, source(&session), name, 67108864);
                match (by_path, by_session) {
                    (Ok(a), Ok(b)) => assert!(a.eq(b).unwrap(), "{name}"),
                    (Err(a), Err(b)) => {
                        assert!(a.get_type(py).is(b.get_type(py)), "{name}");
                        assert_eq!(a.to_string(), b.to_string(), "{name}");
                    }
                    (a, b) => panic!("{name}: path {a:?} versus session {b:?}"),
                }
                let a = typed_dataset_attributes(py, source(&path), name, 67108864).unwrap();
                let b = typed_dataset_attributes(py, source(&session), name, 67108864).unwrap();
                assert!(a.eq(b).unwrap(), "{name}");
            }
            let a = typed_group_attributes(py, source(&path), "/").unwrap();
            let b = typed_group_attributes(py, source(&session), "/").unwrap();
            assert_eq!(a.len(), b.len());
            for (x, y) in a.iter().zip(&b) {
                assert!(x.eq(y).unwrap());
            }
            // A closed session is refused with the session's own error, never reopened.
            session.call_method0("close").unwrap();
            let closed = typed_group_attributes(py, source(&session), "/").unwrap_err();
            assert!(closed.is_instance_of::<pyo3::exceptions::PyValueError>(py));
            assert!(closed.to_string().contains("HDF session is closed"));
            // Anything that is neither a session nor a path keeps the path TypeError.
            let wrong = 5i64.into_pyobject(py).unwrap().into_any();
            let error = match wrong.extract::<Source>() {
                Ok(_) => panic!("an integer is not a source"),
                Err(e) => e,
            };
            let expected = wrong.extract::<PathBuf>().unwrap_err();
            assert!(error.get_type(py).is(expected.get_type(py)));
            assert_eq!(error.to_string(), expected.to_string());
        });
    }
}
