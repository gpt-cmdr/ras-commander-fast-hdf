//! Engine API generation and the machine-readable list of public entry points, so bindings can
//! report what the pinned engine offers instead of hand-writing it.

/// Engine public-API generation. It advances for breaking changes and additive capability
/// generations (such as API 8 and 9), independently of any binding's own API number. API 10
/// changes packed mesh ring output (the public ring-order contract, see `docs/reference/engine-api.md`).
/// See the API version history in `docs/reference/engine-api.md`.
pub const API_VERSION: u32 = 10;

/// One public entry point: a method or associated function and the Cargo feature it needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct Entrypoint {
    /// Type that owns the function (`ReadOnlyHdfSession`, `Topology`, `crs`, `catalog`).
    pub owner: &'static str,
    pub name: &'static str,
    /// Smallest feature that enables it: `core` (always compiled), `catalog`, `hdf`, `mesh` or
    /// `results`.
    pub feature: &'static str,
}

const fn e(owner: &'static str, name: &'static str, feature: &'static str) -> Entrypoint {
    Entrypoint {
        owner,
        name,
        feature,
    }
}

/// Every non-deprecated public entry point of the engine, by owner. A test fails if a public
/// `ReadOnlyHdfSession` method is added or removed without updating this list.
pub const ENTRYPOINTS: &[Entrypoint] = &[
    e("ReadOnlyHdfSession", "with_cache_options", "hdf"),
    e("ReadOnlyHdfSession", "chunk_cache_stats", "hdf"),
    e("ReadOnlyHdfSession", "read_mesh_summary_values", "hdf"),
    e("ReadOnlyHdfSession", "read_mesh_boundary_batch", "hdf"),
    e("ReadOnlyHdfSession", "mesh_boundary_links", "hdf"),
    e("ReadOnlyHdfSession", "run_summary", "hdf"),
    e("ReadOnlyHdfSession", "reduce_mesh_series", "hdf"),
    e("ReadOnlyHdfSession", "mesh_boundary_inventory", "hdf"),
    e("ReadOnlyHdfSession", "read_mesh_boundary_series", "hdf"),
    e("ReadOnlyHdfSession", "mesh_series_inventory", "hdf"),
    e("ReadOnlyHdfSession", "read_mesh_series", "hdf"),
    e("ReadOnlyHdfSession", "mesh_summary_inventory", "hdf"),
    e("ReadOnlyHdfSession", "read_mesh_summary", "hdf"),
    e("catalog", "Plan::open", "catalog"),
    e("catalog", "Plan::from_document", "catalog"),
    e("catalog", "ProjectCatalog::open", "catalog"),
    e("catalog", "SourceDocument::read", "catalog"),
    e("catalog", "SourceDocument::from_bytes", "catalog"),
    e("crs", "combine", "core"),
    e("crs", "relative_projection", "core"),
    e("crs", "catalog_projection", "catalog"),
    e("ReadOnlyHdfSession", "open", "hdf"),
    e("ReadOnlyHdfSession", "with_options", "hdf"),
    e("ReadOnlyHdfSession", "from_bytes", "hdf"),
    e("ReadOnlyHdfSession", "thread_count", "hdf"),
    e("ReadOnlyHdfSession", "path", "hdf"),
    e("ReadOnlyHdfSession", "group_members", "hdf"),
    e("ReadOnlyHdfSession", "group_attributes", "hdf"),
    e("ReadOnlyHdfSession", "dataset_attributes", "hdf"),
    e("ReadOnlyHdfSession", "read_strings", "hdf"),
    e("ReadOnlyHdfSession", "numeric_capability", "hdf"),
    e("ReadOnlyHdfSession", "read_numeric", "hdf"),
    e("ReadOnlyHdfSession", "read_numeric_batch", "hdf"),
    e("ReadOnlyHdfSession", "read_numeric_bounded", "hdf"),
    e("ReadOnlyHdfSession", "read_numeric_batch_bounded", "hdf"),
    e("ReadOnlyHdfSession", "typed_group_attributes", "hdf"),
    e("ReadOnlyHdfSession", "typed_dataset_attributes", "hdf"),
    e("ReadOnlyHdfSession", "raw_fixed_strings", "hdf"),
    e("ReadOnlyHdfSession", "simple_compound", "hdf"),
    e("ReadOnlyHdfSession", "raw_crs_evidence", "hdf"),
    e("ReadOnlyHdfSession", "is_steady_plan", "hdf"),
    e("ReadOnlyHdfSession", "plan_result_mode", "hdf"),
    e("ReadOnlyHdfSession", "cross_sections", "hdf"),
    e("ReadOnlyHdfSession", "steady_profile_names", "hdf"),
    e("ReadOnlyHdfSession", "read_cross_section_result", "hdf"),
    e("ReadOnlyHdfSession", "plan_capabilities", "hdf"),
    e("ReadOnlyHdfSession", "structure_inventory", "hdf"),
    e("ReadOnlyHdfSession", "read_structure_series", "hdf"),
    e("ReadOnlyHdfSession", "time_axis", "hdf"),
    e("ReadOnlyHdfSession", "unsteady_time_axis", "hdf"),
    e("ReadOnlyHdfSession", "unsteady_row_times", "hdf"),
    e("ReadOnlyHdfSession", "dataset_units", "hdf"),
    e("ReadOnlyHdfSession", "dataset_units_attribute", "hdf"),
    e("ReadOnlyHdfSession", "unit_system_evidence", "hdf"),
    e("ReadOnlyHdfSession", "dataset_units_bounded", "hdf"),
    e(
        "ReadOnlyHdfSession",
        "dataset_units_attribute_bounded",
        "hdf",
    ),
    e("ReadOnlyHdfSession", "unit_system_evidence_bounded", "hdf"),
    e("ReadOnlyHdfSession", "family_presence", "hdf"),
    e("ReadOnlyHdfSession", "layer_family_presence", "hdf"),
    e("LayerFamily", "as_str", "hdf"),
    e("ReadOnlyHdfSession", "result_units", "hdf"),
    e("ReadOnlyHdfSession", "mesh_names", "hdf"),
    e("ReadOnlyHdfSession", "mesh_name_diagnostics", "hdf"),
    e("ReadOnlyHdfSession", "mesh_inventory", "hdf"),
    e("ReadOnlyHdfSession", "read_topology", "mesh"),
    e("ReadOnlyHdfSession", "read_topology_bounded", "mesh"),
    e("ReadOnlyHdfSession", "read_mesh_perimeter", "mesh"),
    e("ReadOnlyHdfSession", "read_mesh_face_lines", "mesh"),
    e("ReadOnlyHdfSession", "reduce_temporal", "results"),
    e("Topology", "assemble", "mesh"),
    e("Topology", "assemble_with_session", "mesh"),
    e("Topology", "face_lines", "mesh"),
    e("ReadOnlyHdfSession", "mesh_geometry", "hdf"),
    e("ReadOnlyHdfSession", "read_mesh_geometry", "hdf"),
    e("ReadOnlyHdfSession", "mesh_geometry_tables", "hdf"),
    e("ReadOnlyHdfSession", "geometry_family", "hdf"),
    e("ReadOnlyHdfSession", "geometry_family_capability", "hdf"),
    e("ReadOnlyHdfSession", "compare_mesh_identity", "hdf"),
    e("ReadOnlyHdfSession", "layer_inventory", "hdf"),
    e("ReadOnlyHdfSession", "read_mesh_geometry_cached", "hdf"),
    e("MeshGeometryField", "as_str", "hdf"),
    e("MeshGeometryField", "dataset_name", "hdf"),
    e("GeometryFamily", "as_str", "hdf"),
    e("GeometryFamily", "group_path", "hdf"),
    e("MeshMismatchKind", "as_str", "hdf"),
    // API 7: 1D cross-section geometry, line families and topology.
    e("ReadOnlyHdfSession", "xs_geometry_inventory", "hdf"),
    e("ReadOnlyHdfSession", "read_xs_geometry", "hdf"),
    e("ReadOnlyHdfSession", "one_d_line_family", "hdf"),
    e("ReadOnlyHdfSession", "river_reaches", "hdf"),
    e("ReadOnlyHdfSession", "river_junctions", "hdf"),
    e("ReadOnlyHdfSession", "xs_interpolation_surfaces", "hdf"),
    e("XsGeometryField", "as_str", "hdf"),
    e("XsGeometryField", "dataset_names", "hdf"),
    e("OneDLineFamily", "as_str", "hdf"),
    e("OneDLineFamily", "group_path", "hdf"),
    e("OneDLineFamily", "prefix", "hdf"),
    // API 7: every stored steady/unsteady XS result variable.
    e("ReadOnlyHdfSession", "xs_result_inventory", "hdf"),
    e("ReadOnlyHdfSession", "read_xs_result", "hdf"),
    e("ReadOnlyHdfSession", "unsteady_row_window", "hdf"),
    e("XsResultDomain", "as_str", "hdf"),
    // API 7: 1D and structure Summary Output.
    e("ReadOnlyHdfSession", "one_d_summary_inventory", "hdf"),
    e("ReadOnlyHdfSession", "read_one_d_summary", "hdf"),
    e("OneDElementDomain", "as_str", "hdf"),
    // API 7: 1D structures, structure/pump/storage geometry groups.
    e("ReadOnlyHdfSession", "one_d_geometry_group", "hdf"),
    e("ReadOnlyHdfSession", "structure_geometry", "hdf"),
    e("ReadOnlyHdfSession", "one_d_structure_inventory", "hdf"),
    e("ReadOnlyHdfSession", "one_d_structure_capabilities", "hdf"),
    e("ReadOnlyHdfSession", "read_one_d_structure_series", "hdf"),
    e("ReadOnlyHdfSession", "pump_stations", "hdf"),
    e("OneDStructureKind", "as_str", "hdf"),
    e("OneDStructureKind", "group_name", "hdf"),
    e("OneDStructureKind", "geometry_table", "hdf"),
    e("OneDStructureKind", "group_path", "hdf"),
    e("OneDGeometryGroup", "as_str", "hdf"),
    e("OneDGeometryGroup", "group_path", "hdf"),
    e("OneDStructureKind", "qualified", "hdf"),
    e("ReadOnlyHdfSession", "classification_catalogue", "hdf"),
    e("ReadOnlyHdfSession", "output_block_inventory", "hdf"),
    e("ReadOnlyHdfSession", "read_output_block", "hdf"),
    e("ReadOnlyHdfSession", "read_family_inventory", "hdf"),
    e("ReadOnlyHdfSession", "read_family_dataset", "hdf"),
    e("ReadOnlyHdfSession", "reference_point_geometry", "hdf"),
    e("ReadOnlyHdfSession", "mesh_corner_elevations", "hdf"),
    // Additive within API 10 (terrio HECRAS driver; not called by ras-commander).
    e("ReadOnlyHdfSession", "hec_ras_file_type", "hdf"),
    e("ReadOnlyHdfSession", "hec_ras_file_type_of_bytes", "hdf"),
    e("StoredFileType", "is_hec_ras", "hdf"),
    e("ReadOnlyHdfSession", "pipe_nodes", "hdf"),
    // Additive within API 10: reference results joined to their geometry features.
    e("ReadOnlyHdfSession", "reference_line_results", "hdf"),
    e("ReadOnlyHdfSession", "reference_point_results", "hdf"),
];

/// What this build of the engine offers.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct EngineCapabilities {
    pub api_version: u32,
    /// Cargo features compiled into this build.
    pub features: Vec<&'static str>,
    /// Entry points available with those features.
    pub entrypoints: Vec<Entrypoint>,
}

/// Capabilities of this build: API generation, compiled features and the entry points they
/// enable. Entry points of a disabled feature are absent, not listed as unavailable.
pub fn capabilities() -> EngineCapabilities {
    let mut features = Vec::new();
    if cfg!(feature = "catalog") {
        features.push("catalog");
    }
    if cfg!(feature = "hdf") {
        features.push("hdf");
    }
    if cfg!(feature = "mesh") {
        features.push("mesh");
    }
    if cfg!(feature = "results") {
        features.push("results");
    }
    let entrypoints = ENTRYPOINTS
        .iter()
        .copied()
        .filter(|e| e.feature == "core" || features.contains(&e.feature))
        .collect();
    EngineCapabilities {
        api_version: API_VERSION,
        features,
        entrypoints,
    }
}
