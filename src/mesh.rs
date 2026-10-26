//! Source-exact mesh construction. Only proven invalid geometry may be quarantined.
use crate::predicates::Point;
use crate::predicates::{orient2d, segment_contact, Contact, Orientation, Segment};
use crate::{Error, ErrorKind};
use ndarray::Array2;
use rayon::prelude::*;
use std::collections::HashMap;

/// Validate source geometry without normalizing/removing any output vertices.
/// Zero-length edges are ignored in this validation view, so arbitrary caller rings with
/// repeated adjacent coordinates are accepted. Packed mesh rings never contain them (ring
/// assembly drops consecutive duplicates first); forward collinear perimeter vertices remain.
pub(crate) fn validate_ring(points: impl IntoIterator<Item = Point>) -> Result<(), String> {
    let mut points = points.into_iter();
    let first = points.next().ok_or("empty cell ring")?;
    if !first.iter().all(|v| v.is_finite()) {
        return Err("nonfinite cell ring coordinate".into());
    }
    let mut previous = first;
    let mut edges = Vec::new();
    for point in points {
        if !point.iter().all(|v| v.is_finite()) {
            return Err("nonfinite cell ring coordinate".into());
        }
        if point != previous {
            edges.push(Segment::new(previous, point));
        }
        previous = point;
    }
    if previous != first {
        return Err("unclosed cell ring".into());
    }
    if edges.len() < 3 {
        return Err("zero-area cell ring (fewer than three nonzero edges)".into());
    }
    // Avoid a shoelace sum: cancellation at large projected coordinate origins
    // can misclassify a valid narrow cell. A simple non-collinear closed ring
    // has nonzero area; robust orientation plus intersection checks proves it.
    if edges
        .iter()
        .skip(1)
        .all(|edge| orient2d(edges[0].start, edges[0].end, edge.end) == Orientation::Collinear)
    {
        return Err("zero-area (collinear) cell ring".into());
    }
    for i in 0..edges.len() {
        for j in i + 1..edges.len() {
            // Disjoint bounding boxes are rejected before robust orientation.
            let Some(contact) = segment_contact(edges[i], edges[j]) else {
                continue;
            };
            let adjacent = j == i + 1 || (i == 0 && j + 1 == edges.len());
            if !adjacent {
                return Err("cell ring self-intersection or nonadjacent touch".into());
            }
            if contact == Contact::Collinear {
                return Err("cell ring backtracks over an adjacent edge".into());
            }
        }
    }
    Ok(())
}
// Only a proven geometric invalidity is eligible for quarantine. Bounds,
// connectivity, layout and arithmetic failures never enter that policy.
#[derive(Debug)]
enum CellError {
    Structural(String),
    Geometry(String),
}
impl From<String> for CellError {
    fn from(value: String) -> Self {
        Self::Structural(value)
    }
}
impl From<&str> for CellError {
    fn from(value: &str) -> Self {
        Self::Structural(value.into())
    }
}
impl std::fmt::Display for CellError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Structural(s) | Self::Geometry(s) => f.write_str(s),
        }
    }
}

fn index(n: i64, bound: usize, label: &str) -> Result<usize, String> {
    let n = usize::try_from(n).map_err(|_| format!("negative {label}"))?;
    if n >= bound {
        return Err(format!("{label} {n} out of range {bound}"));
    }
    Ok(n)
}
fn span(
    start: i64,
    count: i64,
    bound: usize,
    label: &str,
) -> Result<std::ops::Range<usize>, String> {
    let start = usize::try_from(start).map_err(|_| format!("negative {label} offset"))?;
    let count = usize::try_from(count).map_err(|_| format!("negative {label} count"))?;
    let end = start
        .checked_add(count)
        .ok_or_else(|| format!("overflow in {label}"))?;
    if end > bound {
        return Err(format!("{label} range {start}..{end} exceeds {bound}"));
    }
    Ok(start..end)
}

#[non_exhaustive]
pub struct Topology {
    pub points: Array2<f64>,
    pub ends: Array2<i64>,
    pub perim_info: Array2<i64>,
    pub perim: Array2<f64>,
    pub cell_info: Array2<i64>,
    pub cell_faces: Array2<i64>,
}
impl Topology {
    pub fn new(
        points: Array2<f64>,
        ends: Array2<i64>,
        perim_info: Array2<i64>,
        perim: Array2<f64>,
        cell_info: Array2<i64>,
        cell_faces: Array2<i64>,
    ) -> Self {
        Self {
            points,
            ends,
            perim_info,
            perim,
            cell_info,
            cell_faces,
        }
    }

    fn validate(&self) -> Result<(), String> {
        for (label, array) in [
            ("face points", &self.points),
            ("perimeter coordinates", &self.perim),
        ] {
            if array.ncols() != 2 {
                return Err(format!("{label} must have two columns"));
            }
            if array.iter().any(|v| !v.is_finite()) {
                return Err(format!("nonfinite {label}"));
            }
        }
        for (label, array, min) in [
            ("face endpoints", &self.ends, 2),
            ("perimeter info", &self.perim_info, 2),
            ("cell info", &self.cell_info, 2),
            ("cell faces", &self.cell_faces, 1),
        ] {
            if array.ncols() < min {
                return Err(format!("{label} needs at least {min} columns"));
            }
        }
        if self.ends.nrows() != self.perim_info.nrows() {
            return Err("face/perimeter row mismatch".into());
        }
        for f in 0..self.ends.nrows() {
            index(self.ends[[f, 0]], self.points.nrows(), "face endpoint")?;
            index(self.ends[[f, 1]], self.points.nrows(), "face endpoint")?;
            span(
                self.perim_info[[f, 0]],
                self.perim_info[[f, 1]],
                self.perim.nrows(),
                "face perimeter",
            )?;
        }
        // Preflight every cell before geometry work, including cells following
        // an invalid ring. Corruption cannot be hidden by strict early failure.
        for cell in 0..self.cell_info.nrows() {
            let range = span(
                self.cell_info[[cell, 0]],
                self.cell_info[[cell, 1]],
                self.cell_faces.nrows(),
                "cell faces",
            )
            .map_err(|e| format!("cell {cell}: {e}"))?;
            for row in range {
                index(self.cell_faces[[row, 0]], self.ends.nrows(), "cell face")
                    .map_err(|e| format!("cell {cell}: {e}"))?;
            }
        }
        Ok(())
    }
    /// Assemble one cell's exterior ring under the **public ring-order contract**
    /// (identical to ras-commander's Python `get_mesh_cell_polygons`, byte for byte):
    ///
    /// * the exterior ring runs **clockwise**;
    /// * it starts at the start point of the cell's **first listed face**
    ///   (`Cells Face and Orientation Values[start, 0]`), traversed in the cell's clockwise
    ///   direction: face B to A when the orientation value (column 1) is `+1` (the cell is the
    ///   left cell of A to B, so A to B is counter-clockwise for it) and A to B when it is `-1`;
    /// * the remaining faces follow around the cell, each contributing its
    ///   `Faces Perimeter Values` in traversal order (reversed when the face is walked B to A);
    /// * consecutive duplicate coordinates are dropped, then the ring is closed;
    /// * a cell with fewer than three faces is omitted (`InsufficientFaces`; `ZeroFaces` for none).
    ///
    /// Equivalently: the reverse of HEC-RAS's stored counter-clockwise corner order, rotated so
    /// the first listed face comes first. Only an orientation of `+1` or `-1` selects the start;
    /// any other value (or a missing column) starts at A, and the walked ring is then tested with
    /// robust `orient2d`. A counter-clockwise result is rebuilt from the opposite end of face 0,
    /// so the ring is clockwise whatever the stored orientation column says.
    fn ring(&self, cell: usize) -> Result<Option<Vec<Point>>, CellError> {
        let slice = span(
            self.cell_info[[cell, 0]],
            self.cell_info[[cell, 1]],
            self.cell_faces.nrows(),
            "cell faces",
        )?;
        if slice.is_empty() {
            return Ok(None);
        }
        let first_row = slice.start;
        let faces: Vec<usize> = slice
            .map(|row| index(self.cell_faces[[row, 0]], self.ends.nrows(), "cell face"))
            .collect::<Result<_, _>>()?;
        // A face whose endpoints coincide is corrupt topology at any face count.
        if faces
            .iter()
            .any(|&f| self.ends[[f, 0]] == self.ends[[f, 1]])
        {
            return Err("self-loop face".into());
        }
        // Fewer than three faces cannot bound a cell. HEC-RAS appends boundary ghost rows with
        // a single open face; Python reports `insufficient_faces` for these and for two-face
        // cells alike, so the rule is by listed face count (no cutoff guessed from the
        // real-cell count, no renumbering).
        if faces.len() < 3 {
            return Ok(None);
        }
        if faces
            .iter()
            .enumerate()
            .any(|(i, face)| faces[..i].contains(face))
        {
            return Err("duplicate face in cell boundary".into());
        }
        let mut adjacency: HashMap<i64, Vec<usize>> = HashMap::with_capacity(faces.len());
        for (i, &f) in faces.iter().enumerate() {
            let a = self.ends[[f, 0]];
            let b = self.ends[[f, 1]];
            adjacency.entry(a).or_default().push(i);
            adjacency.entry(b).or_default().push(i);
        }
        if adjacency.values().any(|v| v.len() != 2) {
            return Err("cell boundary is not a degree-two closed chain".into());
        }
        let a0 = self.ends[[faces[0], 0]];
        let b0 = self.ends[[faces[0], 1]];
        let orientation = if self.cell_faces.ncols() > 1 {
            self.cell_faces[[first_row, 1]]
        } else {
            0
        };
        // Orientation +1: the cell is left of A to B, so its clockwise direction is B to A.
        let (first, alternate) = if orientation == 1 { (b0, a0) } else { (a0, b0) };
        let mut ring = self.walk(&faces, &adjacency, first)?;
        if winding(&ring) == Orientation::CounterClockwise {
            ring = self.walk(&faces, &adjacency, alternate)?;
        }
        // Validation sees exactly the emitted (deduplicated) ring.
        validate_ring(ring.iter().copied()).map_err(CellError::Geometry)?;
        Ok(Some(ring))
    }
    /// Walk the degree-two face chain from `first`, taking face 0 first, appending perimeter
    /// values in traversal order and skipping consecutive duplicate coordinates.
    fn walk(
        &self,
        faces: &[usize],
        adjacency: &HashMap<i64, Vec<usize>>,
        first: i64,
    ) -> Result<Vec<Point>, CellError> {
        fn push(ring: &mut Vec<Point>, p: Point) {
            if ring.last() != Some(&p) {
                ring.push(p);
            }
        }
        let mut point = first;
        let mut visited = vec![false; faces.len()];
        let mut ring = vec![[
            self.points[[first as usize, 0]],
            self.points[[first as usize, 1]],
        ]];
        for _ in 0..faces.len() {
            let local = *adjacency
                .get(&point)
                .and_then(|v| v.iter().find(|&&i| !visited[i]))
                .ok_or("disconnected or repeated cell boundary")?;
            visited[local] = true;
            let face = faces[local];
            let reverse = self.ends[[face, 1]] == point;
            let range = span(
                self.perim_info[[face, 0]],
                self.perim_info[[face, 1]],
                self.perim.nrows(),
                "face perimeter",
            )?;
            if reverse {
                for row in range.rev() {
                    push(&mut ring, [self.perim[[row, 0]], self.perim[[row, 1]]]);
                }
            } else {
                for row in range {
                    push(&mut ring, [self.perim[[row, 0]], self.perim[[row, 1]]]);
                }
            }
            point = self.ends[[face, if reverse { 0 } else { 1 }]];
            push(
                &mut ring,
                [
                    self.points[[point as usize, 0]],
                    self.points[[point as usize, 1]],
                ],
            );
        }
        if point != first {
            return Err("unclosed cell boundary".into());
        }
        Ok(ring)
    }
}

/// Robust winding of a closed ring, read at its lowest-then-leftmost vertex (always a convex
/// hull vertex, so a simple ring never reads collinear there). `Collinear` means no decision:
/// the ring is degenerate and validation rejects it.
fn winding(ring: &[Point]) -> Orientation {
    let n = ring.len().saturating_sub(1);
    if n < 3 {
        return Orientation::Collinear;
    }
    let lower = |a: &Point, b: &Point| a[0] < b[0] || (a[0] == b[0] && a[1] < b[1]);
    let mut i = 0;
    for k in 1..n {
        if lower(&ring[k], &ring[i]) {
            i = k;
        }
    }
    let v = ring[i];
    let Some(prev) = (1..n).map(|d| ring[(i + n - d) % n]).find(|p| *p != v) else {
        return Orientation::Collinear;
    };
    let Some(next) = (1..n).map(|d| ring[(i + d) % n]).find(|p| *p != v) else {
        return Orientation::Collinear;
    };
    orient2d(prev, v, next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::array;

    fn four_face_cell(points: Array2<f64>) -> Topology {
        Topology {
            points,
            ends: array![[0, 1], [1, 2], [2, 3], [3, 0]],
            perim_info: Array2::zeros((4, 2)),
            perim: Array2::zeros((0, 2)),
            cell_info: array![[0, 4]],
            cell_faces: array![[0], [1], [2], [3]],
        }
    }

    #[test]
    fn four_face_bowtie_is_not_a_valid_source_polygon() {
        let top = four_face_cell(array![[0., 0.], [2., 2.], [0., 2.], [2., 0.]]);
        top.validate().unwrap();
        assert!(top
            .ring(0)
            .unwrap_err()
            .to_string()
            .contains("self-intersection"));
    }

    #[test]
    fn four_face_collinear_closed_chain_has_zero_area() {
        let top = four_face_cell(array![[0., 0.], [1., 0.], [2., 0.], [3., 0.]]);
        top.validate().unwrap();
        assert!(top.ring(0).unwrap_err().to_string().contains("zero-area"));
    }

    #[test]
    fn nonadjacent_touch_and_backtracking_are_rejected() {
        assert!(validate_ring([
            [0., 0.],
            [2., 0.],
            [1., 1.],
            [2., 2.],
            [0., 2.],
            [1., 1.],
            [0., 0.]
        ])
        .is_err());
        assert!(
            validate_ring([[0., 0.], [2., 0.], [1., 0.], [1., 2.], [0., 2.], [0., 0.]]).is_err()
        );
    }

    #[test]
    fn ring_drops_consecutive_duplicates_and_keeps_collinear_vertices() {
        let mut top = four_face_cell(array![[0., 0.], [2., 0.], [2., 2.], [0., 2.]]);
        top.perim_info = array![[0, 3], [3, 0], [3, 0], [3, 0]];
        top.perim = array![[0., 0.], [1., 0.], [1., 0.]];
        top.validate().unwrap();
        let ring = top.ring(0).unwrap().unwrap();
        // Face 0 walked B to A: the repeated (1,0) and the perimeter's repeat of the A point
        // collapse, while the collinear (1,0) vertex stays.
        assert_eq!(
            ring,
            vec![[2., 0.], [1., 0.], [0., 0.], [0., 2.], [2., 2.], [2., 0.]]
        );
    }

    /// The contract ring of the 2 x 2 square below when face 0 is listed first.
    const CW_SQUARE_FROM_B0: [[f64; 2]; 5] = [[2., 0.], [0., 0.], [0., 2.], [2., 2.], [2., 0.]];
    /// Square whose faces are stored counter-clockwise (A to B), the stored HEC-RAS order.
    fn ccw_quad(cell_faces: Array2<i64>) -> Topology {
        Topology {
            cell_faces,
            ..four_face_cell(array![[0., 0.], [2., 0.], [2., 2.], [0., 2.]])
        }
    }

    #[test]
    fn orientation_plus_one_starts_at_b_of_the_first_face_and_runs_clockwise() {
        // Every face is A to B with the cell on its left: orientation +1.
        let top = ccw_quad(array![[0, 1], [1, 1], [2, 1], [3, 1]]);
        top.validate().unwrap();
        assert_eq!(top.ring(0).unwrap().unwrap(), CW_SQUARE_FROM_B0);
    }

    #[test]
    fn orientation_minus_one_starts_at_a_of_the_first_face_and_runs_clockwise() {
        // The same square with every face reversed: A to B is already clockwise for the cell.
        let top = Topology {
            points: array![[0., 0.], [2., 0.], [2., 2.], [0., 2.]],
            ends: array![[1, 0], [2, 1], [3, 2], [0, 3]],
            perim_info: Array2::zeros((4, 2)),
            perim: Array2::zeros((0, 2)),
            cell_info: array![[0, 4]],
            cell_faces: array![[0, -1], [1, -1], [2, -1], [3, -1]],
        };
        top.validate().unwrap();
        assert_eq!(top.ring(0).unwrap().unwrap(), CW_SQUARE_FROM_B0);
    }

    #[test]
    fn the_first_listed_face_selects_the_start_not_the_lowest_face_id() {
        // Listed from face 2: start at B of face 2, (0,2), and run clockwise.
        let top = ccw_quad(array![[2, 1], [3, 1], [0, 1], [1, 1]]);
        top.validate().unwrap();
        assert_eq!(
            top.ring(0).unwrap().unwrap(),
            vec![[0., 2.], [2., 2.], [2., 0.], [0., 0.], [0., 2.]]
        );
    }

    #[test]
    fn unexpected_or_missing_orientation_still_yields_the_clockwise_contract_ring() {
        // Missing column, zero, an unknown value, and a column that contradicts the
        // geometry (-1 on a counter-clockwise listing) all fall back to orient2d on the
        // walked ring, so the result is the contract ring in every case.
        for cell_faces in [
            array![[0], [1], [2], [3]],
            array![[0, 0], [1, 0], [2, 0], [3, 0]],
            array![[0, 7], [1, 7], [2, 7], [3, 7]],
            array![[0, -1], [1, -1], [2, -1], [3, -1]],
        ] {
            let top = ccw_quad(cell_faces.clone());
            top.validate().unwrap();
            assert_eq!(
                top.ring(0).unwrap().unwrap(),
                CW_SQUARE_FROM_B0,
                "{cell_faces:?}"
            );
        }
        // A clockwise listing with a missing column is kept as walked (start at A of face 0).
        let mut top = Topology {
            points: array![[0., 0.], [2., 0.], [2., 2.], [0., 2.]],
            ends: array![[1, 0], [2, 1], [3, 2], [0, 3]],
            perim_info: Array2::zeros((4, 2)),
            perim: Array2::zeros((0, 2)),
            cell_info: array![[0, 4]],
            cell_faces: array![[0], [1], [2], [3]],
        };
        top.validate().unwrap();
        assert_eq!(top.ring(0).unwrap().unwrap(), CW_SQUARE_FROM_B0);
        top.cell_faces = array![[0, 9], [1, 9], [2, 9], [3, 9]];
        assert_eq!(top.ring(0).unwrap().unwrap(), CW_SQUARE_FROM_B0);
    }

    #[test]
    fn curved_perimeter_values_follow_the_clockwise_traversal() {
        let mut top = ccw_quad(array![[0, 1], [1, 1], [2, 1], [3, 1]]);
        // Face 0, (0,0) to (2,0), bulges through (0.5,-0.5) then (1.5,-0.5); face 2, stored
        // (2,2) to (0,2), passes (1,3). The clockwise ring walks both faces B to A, so each
        // face's perimeter values appear reversed.
        top.perim_info = array![[0, 2], [2, 0], [2, 1], [3, 0]];
        top.perim = array![[0.5, -0.5], [1.5, -0.5], [1., 3.]];
        top.validate().unwrap();
        assert_eq!(
            top.ring(0).unwrap().unwrap(),
            vec![
                [2., 0.],
                [1.5, -0.5],
                [0.5, -0.5],
                [0., 0.],
                [0., 2.],
                [1., 3.],
                [2., 2.],
                [2., 0.]
            ]
        );
    }

    #[test]
    fn two_face_cells_are_insufficient_not_polygons() {
        let top = Topology {
            points: array![[0., 0.], [1., 0.]],
            ends: array![[0, 1], [1, 0]],
            perim_info: array![[0, 1], [1, 1]],
            perim: array![[0.5, 1.], [0.5, -1.]],
            cell_info: array![[0, 2]],
            cell_faces: array![[0, 1], [1, 1]],
        };
        top.validate().unwrap();
        assert!(top.ring(0).unwrap().is_none());
        let packed = top.assemble(GeometryPolicy::Strict, 1).unwrap();
        assert!(packed.cell_ids.is_empty());
        assert_eq!(packed.diagnostics[0].kind, OmissionKind::InsufficientFaces);
        assert_eq!(packed.diagnostics[0].reason, "2 faces");
    }

    #[test]
    fn narrow_cell_at_large_projected_origin_is_valid() {
        let top = four_face_cell(array![
            [1e9, 1e9],
            [1e9 + 1., 1e9],
            [1e9 + 1., 1e9 + 1e-6],
            [1e9, 1e9 + 1e-6]
        ]);
        assert!(top.ring(0).unwrap().is_some());
    }
    #[test]
    fn curved_reversed_face_is_preserved() {
        let top = Topology {
            points: array![[0., 0.], [2., 0.], [2., 2.], [0., 2.]],
            ends: array![[0, 1], [2, 1], [2, 3], [3, 0]],
            perim_info: array![[0, 0], [0, 1], [1, 0], [1, 0]],
            perim: array![[3., 1.]],
            cell_info: array![[0, 4]],
            cell_faces: array![[2, 0], [0, 0], [3, 0], [1, 0]],
        };
        top.validate().unwrap();
        let ring = top.ring(0).unwrap().unwrap();
        assert_eq!(ring.first(), ring.last());
        assert_eq!(ring.len(), 6);
        assert!(ring.contains(&[3., 1.]));
    }

    #[test]
    fn disconnected_cycles_are_rejected() {
        let top = Topology {
            points: array![[0., 0.], [1., 0.], [0., 1.], [3., 0.], [4., 0.], [3., 1.]],
            ends: array![[0, 1], [1, 2], [2, 0], [3, 4], [4, 5], [5, 3]],
            perim_info: Array2::zeros((6, 2)),
            perim: Array2::zeros((0, 2)),
            cell_info: array![[0, 6]],
            cell_faces: array![[0], [1], [2], [3], [4], [5]],
        };
        top.validate().unwrap();
        assert!(top
            .ring(0)
            .unwrap_err()
            .to_string()
            .contains("disconnected"));
    }

    #[test]
    fn invalid_offsets_fail_without_indexing() {
        assert!(span(i64::MAX, i64::MAX, 2, "test").is_err());
        assert!(span(-1, 1, 2, "test").is_err());
        assert!(index(-1, 2, "test").is_err());
        assert!(index(2, 2, "test").is_err());
    }

    #[test]
    fn single_open_face_ghost_is_omitted() {
        let top = Topology {
            points: array![[0., 0.], [1., 0.]],
            ends: array![[0, 1]],
            perim_info: array![[0, 0]],
            perim: Array2::zeros((0, 2)),
            cell_info: array![[0, 1]],
            cell_faces: array![[0, -1]],
        };
        top.validate().unwrap();
        assert!(top.ring(0).unwrap().is_none());
    }

    #[test]
    fn duplicated_curved_face_cannot_fabricate_a_polygon() {
        let top = Topology {
            points: array![[0., 0.], [1., 0.], [0., 1.]],
            ends: array![[0, 1], [1, 2], [2, 0]],
            perim_info: array![[0, 1], [1, 0], [1, 0]],
            perim: array![[0.5, -1.]],
            cell_info: array![[0, 4]],
            cell_faces: array![[0, 1], [1, 1], [2, 1], [0, -1]],
        };
        top.validate().unwrap();
        assert!(top
            .ring(0)
            .unwrap_err()
            .to_string()
            .contains("duplicate face"));
    }
}

/// Explicit geometry policy. Structural corruption is fatal in both modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum GeometryPolicy {
    Strict,
    QuarantineInvalid,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum OmissionKind {
    InvalidGeometry,
    ZeroFaces,
    InsufficientFaces,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CellDiagnostic {
    pub cell_id: i64,
    pub kind: OmissionKind,
    pub reason: String,
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct PackedMesh {
    pub coordinates: Vec<[f64; 2]>,
    pub offsets: Vec<u64>,
    pub cell_ids: Vec<i64>,
    pub diagnostics: Vec<CellDiagnostic>,
}
impl Topology {
    /// Standalone assembly owns a new bounded pool. Prefer assemble_with_session
    /// when a read session already exists.
    pub fn assemble(&self, policy: GeometryPolicy, threads: usize) -> crate::Result<PackedMesh> {
        self.validate()
            .map_err(|e| Error::new(ErrorKind::StructuralTopology, e))?;
        let pool = crate::SessionPool::from(crate::pool(threads)?);
        self.assemble_on_pool(policy, &pool, true)
    }
    /// Reuse exactly the session pool; session parallel=false executes rings serially.
    pub fn assemble_with_session(
        &self,
        policy: GeometryPolicy,
        session: &crate::hdf::ReadOnlyHdfSession,
    ) -> crate::Result<PackedMesh> {
        self.validate()
            .map_err(|e| Error::new(ErrorKind::StructuralTopology, e))?;
        self.assemble_on_pool(policy, &session.pool, session.parallel_enabled())
    }
    fn assemble_on_pool(
        &self,
        policy: GeometryPolicy,
        pool: &crate::SessionPool,
        parallel: bool,
    ) -> crate::Result<PackedMesh> {
        let rings: Vec<_> = if parallel {
            pool.install(|| {
                (0..self.cell_info.nrows())
                    .into_par_iter()
                    .map(|c| self.ring(c))
                    .collect()
            })
        } else {
            (0..self.cell_info.nrows()).map(|c| self.ring(c)).collect()
        };
        let mut out = PackedMesh {
            coordinates: Vec::new(),
            offsets: vec![0],
            cell_ids: Vec::new(),
            diagnostics: Vec::new(),
        };
        for (cell, ring) in rings.into_iter().enumerate() {
            let id = i64::try_from(cell)
                .map_err(|_| Error::new(ErrorKind::StructuralTopology, "cell ID overflow"))?;
            match ring {
                Ok(Some(ring)) => {
                    out.coordinates.extend(ring);
                    out.offsets
                        .push(u64::try_from(out.coordinates.len()).map_err(|_| {
                            Error::new(
                                ErrorKind::StructuralTopology,
                                "coordinate count exceeds u64",
                            )
                        })?);
                    out.cell_ids.push(id);
                }
                Ok(None) => {
                    let count = self.cell_info[[cell, 1]];
                    out.diagnostics.push(CellDiagnostic {
                        cell_id: id,
                        kind: if count == 0 {
                            OmissionKind::ZeroFaces
                        } else {
                            OmissionKind::InsufficientFaces
                        },
                        reason: format!("{count} faces"),
                    });
                }
                Err(CellError::Geometry(reason)) if policy == GeometryPolicy::QuarantineInvalid => {
                    out.diagnostics.push(CellDiagnostic {
                        cell_id: id,
                        kind: OmissionKind::InvalidGeometry,
                        reason,
                    })
                }
                Err(CellError::Geometry(reason)) => {
                    return Err(Error::new(
                        ErrorKind::InvalidGeometry,
                        format!("cell {id}: {reason}"),
                    ))
                }
                Err(CellError::Structural(reason)) => {
                    return Err(Error::new(
                        ErrorKind::StructuralTopology,
                        format!("cell {id}: {reason}"),
                    ))
                }
            }
        }
        Ok(out)
    }
}
/// Composite identities are source + mesh + original source cell ID, never output row.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct NamedMesh {
    pub source: std::path::PathBuf,
    pub mesh_name: String,
    pub topology: Topology,
}
impl std::fmt::Debug for Topology {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Topology")
            .field("points", &self.points.shape())
            .field("cells", &self.cell_info.shape())
            .finish()
    }
}
impl Clone for Topology {
    fn clone(&self) -> Self {
        Self {
            points: self.points.clone(),
            ends: self.ends.clone(),
            perim_info: self.perim_info.clone(),
            perim: self.perim.clone(),
            cell_info: self.cell_info.clone(),
            cell_faces: self.cell_faces.clone(),
        }
    }
}
impl PartialEq for Topology {
    fn eq(&self, o: &Self) -> bool {
        self.points == o.points
            && self.ends == o.ends
            && self.perim_info == o.perim_info
            && self.perim == o.perim
            && self.cell_info == o.cell_info
            && self.cell_faces == o.cell_faces
    }
}
impl crate::hdf::ReadOnlyHdfSession {
    /// Read and validate a named mesh topology, refusing more than
    /// [`DEFAULT_READ_MAX_BYTES`](crate::hdf::DEFAULT_READ_MAX_BYTES) across
    /// its six arrays before allocating.
    pub fn read_topology(&self, mesh_name: &str) -> crate::Result<NamedMesh> {
        self.read_topology_bounded(mesh_name, crate::hdf::DEFAULT_READ_MAX_BYTES)
    }
    /// [`read_topology`](Self::read_topology) with an explicit budget shared by
    /// the six topology arrays. All sizes are checked before any array is
    /// read, so a declared-huge mesh allocates nothing.
    pub fn read_topology_bounded(
        &self,
        mesh_name: &str,
        max_bytes: usize,
    ) -> crate::Result<NamedMesh> {
        if mesh_name.is_empty() || mesh_name.contains('/') || mesh_name.contains('\\') {
            return Err(Error::new(
                ErrorKind::InvalidRequest,
                "mesh name must be one group component",
            ));
        }
        let names = [
            "FacePoints Coordinate",
            "Faces FacePoint Indexes",
            "Faces Perimeter Info",
            "Faces Perimeter Values",
            "Cells Face and Orientation Info",
            "Cells Face and Orientation Values",
        ];
        let paths = names
            .iter()
            .map(|n| format!("Geometry/2D Flow Areas/{mesh_name}/{n}"))
            .collect::<Vec<_>>();
        let paths = paths.iter().map(String::as_str).collect::<Vec<_>>();
        let mut arrays = self
            .read_numeric_batch_bounded(&paths, max_bytes)?
            .into_iter()
            .map(|(_, array)| array);
        let top = Topology {
            points: as_f64(arrays.next().unwrap())?,
            ends: as_i64(arrays.next().unwrap())?,
            perim_info: as_i64(arrays.next().unwrap())?,
            perim: as_f64(arrays.next().unwrap())?,
            cell_info: as_i64(arrays.next().unwrap())?,
            cell_faces: as_i64(arrays.next().unwrap())?,
        };
        top.validate()
            .map_err(|e| Error::new(ErrorKind::StructuralTopology, e))?;
        Ok(NamedMesh {
            source: self.path().to_path_buf(),
            mesh_name: mesh_name.into(),
            topology: top,
        })
    }
}
/// Face polylines packed like [`PackedMesh`]: line `i` is
/// `coordinates[offsets[i]..offsets[i + 1]]`, for stored face `face_ids[i]`.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct PackedLines {
    pub coordinates: Vec<[f64; 2]>,
    pub offsets: Vec<u64>,
    pub face_ids: Vec<i64>,
}
impl Topology {
    /// Every face as a polyline, in stored face order, exactly as ras-commander's Python
    /// `HdfMesh.get_mesh_cell_faces` builds it: the face's start point
    /// (`FacePoints Coordinate[Faces FacePoint Indexes[f, 0]]`), then its
    /// `Faces Perimeter Values[start..start + count]` rows in stored order, then its end point.
    /// No vertex is dropped, merged or reordered, and no face is skipped. The topology is
    /// validated first (the same structural checks as [`assemble`](Self::assemble)).
    pub fn face_lines(&self) -> crate::Result<PackedLines> {
        self.validate()
            .map_err(|e| Error::new(ErrorKind::StructuralTopology, e))?;
        let faces = self.ends.nrows();
        let mut out = PackedLines {
            coordinates: Vec::new(),
            offsets: Vec::with_capacity(faces + 1),
            face_ids: Vec::with_capacity(faces),
        };
        out.offsets.push(0);
        let point = |i: i64| -> [f64; 2] {
            // In range: validate() checked every endpoint.
            let i = i as usize;
            [self.points[[i, 0]], self.points[[i, 1]]]
        };
        for face in 0..faces {
            out.coordinates.push(point(self.ends[[face, 0]]));
            let range = span(
                self.perim_info[[face, 0]],
                self.perim_info[[face, 1]],
                self.perim.nrows(),
                "face perimeter",
            )
            .map_err(|e| Error::new(ErrorKind::StructuralTopology, e))?;
            for row in range {
                out.coordinates
                    .push([self.perim[[row, 0]], self.perim[[row, 1]]]);
            }
            out.coordinates.push(point(self.ends[[face, 1]]));
            let end = u64::try_from(out.coordinates.len()).map_err(|_| {
                Error::new(
                    ErrorKind::StructuralTopology,
                    "coordinate count exceeds u64",
                )
            })?;
            out.offsets.push(end);
            out.face_ids.push(
                i64::try_from(face)
                    .map_err(|_| Error::new(ErrorKind::StructuralTopology, "face ID overflow"))?,
            );
        }
        Ok(out)
    }
}
impl crate::hdf::ReadOnlyHdfSession {
    /// Every face of a named mesh as a polyline ([`Topology::face_lines`]), reading only the
    /// four face tables (`FacePoints Coordinate`, `Faces FacePoint Indexes`,
    /// `Faces Perimeter Info`, `Faces Perimeter Values`) within `max_bytes`.
    ///
    /// The output is ras-commander's Python `HdfMesh.get_mesh_cell_faces` for that mesh only
    /// where its per-face loop is unambiguous, so every other layout is refused rather than
    /// approximated: each table must be `(rows, 2)` (Unsupported otherwise; Python would
    /// unpack, truncate or build 3D lines), endpoint and perimeter tables must have one row per
    /// face, and every endpoint index and perimeter span must lie inside its table (Python
    /// wraps negative indices and truncates long slices). Nonfinite coordinates are refused
    /// too. Those refusals are StructuralTopology. A missing table is NotFound. The packed
    /// output (`16 * coordinates + 16 * faces` bytes) must also fit within `max_bytes`.
    pub fn read_mesh_face_lines(
        &self,
        mesh_name: &str,
        max_bytes: usize,
    ) -> crate::Result<PackedLines> {
        if mesh_name.is_empty() || mesh_name.contains('/') || mesh_name.contains('\\') {
            return Err(Error::new(
                ErrorKind::InvalidRequest,
                "mesh name must be one group component",
            ));
        }
        let names = [
            "FacePoints Coordinate",
            "Faces FacePoint Indexes",
            "Faces Perimeter Info",
            "Faces Perimeter Values",
        ];
        let paths = names
            .iter()
            .map(|n| format!("Geometry/2D Flow Areas/{mesh_name}/{n}"))
            .collect::<Vec<_>>();
        let refs = paths.iter().map(String::as_str).collect::<Vec<_>>();
        let arrays = self
            .read_numeric_batch_bounded(&refs, max_bytes)?
            .into_iter()
            .map(|(_, array)| array)
            .collect::<Vec<_>>();
        for (path, array) in paths.iter().zip(&arrays) {
            if array.shape.len() != 2 || array.shape[1] != 2 {
                return Err(Error::new(
                    ErrorKind::Unsupported,
                    format!("{path}: face line tables must be (rows, 2)"),
                ));
            }
        }
        let mut arrays = arrays.into_iter();
        let points = as_f64(arrays.next().unwrap())?;
        let ends = as_i64(arrays.next().unwrap())?;
        let perim_info = as_i64(arrays.next().unwrap())?;
        let perim = as_f64(arrays.next().unwrap())?;
        // Output budget before building: start and end point of every face plus each
        // in-range perimeter span (spans are proven in range by face_lines' validation).
        let faces = ends.nrows();
        let interior = perim_info.rows().into_iter().try_fold(0usize, |sum, row| {
            usize::try_from(row[1].max(0))
                .ok()
                .and_then(|n| sum.checked_add(n))
        });
        let out_bytes = interior
            .and_then(|n| n.checked_add(faces.checked_mul(2)?))
            .and_then(|n| n.checked_add(faces))
            .and_then(|n| n.checked_mul(16));
        match out_bytes {
            Some(bytes) if bytes <= max_bytes => {}
            _ => {
                return Err(Error::new(
                    ErrorKind::Unsupported,
                    format!("{mesh_name}: face lines exceed the {max_bytes}-byte read budget"),
                )
                .with_code(crate::UnsupportedCode::ResourceLimit))
            }
        }
        let top = Topology {
            points,
            ends,
            perim_info,
            perim,
            cell_info: Array2::zeros((0, 2)),
            cell_faces: Array2::zeros((0, 1)),
        };
        top.face_lines()
    }
}
/// A 2D flow area's stored perimeter as a closed ring.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct MeshPerimeter {
    pub source: std::path::PathBuf,
    pub mesh_name: String,
    /// The stored `Perimeter` rows in stored order (`f32` widened exactly), with the first
    /// point repeated at the end when the stored rows are not already closed.
    pub ring: Vec<[f64; 2]>,
    /// Rows stored in the `Perimeter` dataset.
    pub stored_points: usize,
}
impl crate::hdf::ReadOnlyHdfSession {
    /// Read `Geometry/2D Flow Areas/<mesh>/Perimeter` as ras-commander's Python
    /// `HdfMesh.get_mesh_areas` does for an area that stores it (`Polygon(rows)`): the stored
    /// rows, closed by repeating the first point when the last differs. Nothing is
    /// simplified, reoriented or validated beyond what that polygon construction requires.
    ///
    /// An absent dataset is NotFound (the Python collection-level `Polygon Info/Parts/Points`
    /// fallback is not implemented). A dataset that is not `(rows, 2)` floats, holds a
    /// nonfinite value or gives fewer than four ring coordinates after closing is Corrupt.
    pub fn read_mesh_perimeter(
        &self,
        mesh_name: &str,
        max_bytes: usize,
    ) -> crate::Result<MeshPerimeter> {
        if mesh_name.is_empty() || mesh_name.contains('/') || mesh_name.contains('\\') {
            return Err(Error::new(
                ErrorKind::InvalidRequest,
                "mesh name must be one group component",
            ));
        }
        let path = format!("Geometry/2D Flow Areas/{mesh_name}/Perimeter");
        let array = self
            .read_numeric_batch_bounded(&[path.as_str()], max_bytes)?
            .into_iter()
            .next()
            .map(|(_, a)| a)
            .ok_or_else(|| Error::new(ErrorKind::Internal, "empty batch result"))?;
        let corrupt = |m: &str| Error::new(ErrorKind::Corrupt, format!("{path}: {m}"));
        if array.shape.len() != 2 || array.shape[1] != 2 {
            return Err(corrupt("perimeter must be (rows, 2)"));
        }
        let values: Vec<f64> = match array.values {
            crate::hdf::NumericValues::F64(v) => v,
            crate::hdf::NumericValues::F32(v) => v.into_iter().map(f64::from).collect(),
            _ => return Err(corrupt("perimeter coordinates must be floats")),
        };
        if values.iter().any(|v| !v.is_finite()) {
            return Err(corrupt("nonfinite perimeter coordinate"));
        }
        let mut ring: Vec<[f64; 2]> = values.as_chunks::<2>().0.to_vec();
        let stored_points = ring.len();
        if let (Some(&first), Some(&last)) = (ring.first(), ring.last()) {
            if first != last {
                ring.push(first);
            }
        }
        if ring.len() < 4 {
            return Err(corrupt("fewer than four ring coordinates after closing"));
        }
        Ok(MeshPerimeter {
            source: self.path().to_path_buf(),
            mesh_name: mesh_name.into(),
            ring,
            stored_points,
        })
    }
}
fn dims(a: &crate::hdf::NumericArray) -> crate::Result<(usize, usize)> {
    if a.shape.len() != 2 {
        return Err(Error::new(
            ErrorKind::StructuralTopology,
            "topology array must be 2D",
        ));
    }
    Ok((
        usize::try_from(a.shape[0])
            .map_err(|_| Error::new(ErrorKind::StructuralTopology, "row overflow"))?,
        usize::try_from(a.shape[1])
            .map_err(|_| Error::new(ErrorKind::StructuralTopology, "column overflow"))?,
    ))
}
fn as_f64(a: crate::hdf::NumericArray) -> crate::Result<Array2<f64>> {
    let d = dims(&a)?;
    let v = match a.values {
        crate::hdf::NumericValues::F64(v) => v,
        crate::hdf::NumericValues::F32(v) => v.into_iter().map(f64::from).collect(),
        _ => {
            return Err(Error::new(
                ErrorKind::StructuralTopology,
                "coordinates must be floats",
            ))
        }
    };
    Array2::from_shape_vec(d, v)
        .map_err(|e| Error::new(ErrorKind::StructuralTopology, e.to_string()))
}
fn as_i64(a: crate::hdf::NumericArray) -> crate::Result<Array2<i64>> {
    let d = dims(&a)?;
    use crate::hdf::NumericValues::*;
    let v = match a.values {
        I8(v) => v.into_iter().map(i64::from).collect(),
        I16(v) => v.into_iter().map(i64::from).collect(),
        I32(v) => v.into_iter().map(i64::from).collect(),
        I64(v) => v,
        U8(v) => v.into_iter().map(i64::from).collect(),
        U16(v) => v.into_iter().map(i64::from).collect(),
        U32(v) => v.into_iter().map(i64::from).collect(),
        U64(v) => v
            .into_iter()
            .map(|v| {
                i64::try_from(v).map_err(|_| {
                    Error::new(ErrorKind::StructuralTopology, "connectivity exceeds int64")
                })
            })
            .collect::<crate::Result<Vec<_>>>()?,
        _ => {
            return Err(Error::new(
                ErrorKind::StructuralTopology,
                "connectivity must contain integers",
            ))
        }
    };
    Array2::from_shape_vec(d, v)
        .map_err(|e| Error::new(ErrorKind::StructuralTopology, e.to_string()))
}

/// Validate a ring without changing source vertices. Geometry errors retain a
/// typed category for binding/tiling consumers; topology preflight remains separate.
pub fn validate_ring_geometry(points: impl IntoIterator<Item = [f64; 2]>) -> crate::Result<()> {
    validate_ring(points).map_err(|reason| Error::new(ErrorKind::InvalidGeometry, reason))
}
#[cfg(test)]
mod public_validation_tests {
    use super::*;
    #[test]
    fn public_ring_validation_preserves_typed_invalid_geometry() {
        validate_ring_geometry([[0., 0.], [1., 0.], [0., 1.], [0., 0.]]).unwrap();
        assert_eq!(
            validate_ring_geometry([[0., 0.], [1., 1.], [0., 1.], [1., 0.], [0., 0.]])
                .unwrap_err()
                .kind,
            ErrorKind::InvalidGeometry
        );
    }
}

#[cfg(test)]
mod session_pool_tests {
    use super::*;
    #[test]
    fn repeated_assembly_reuses_session_pool_and_matches_standalone() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/coverage/coverage_mesh.h5");
        for threads in [1, 4] {
            for parallel in [false, true] {
                let session =
                    crate::hdf::ReadOnlyHdfSession::with_options(&path, threads, parallel).unwrap();
                let top = session.read_topology("Good").unwrap().topology;
                let expected = top.assemble(GeometryPolicy::Strict, 1).unwrap();
                let before = crate::POOL_CREATIONS.with(|n| n.get());
                for _ in 0..4 {
                    assert_eq!(
                        top.assemble_with_session(GeometryPolicy::Strict, &session)
                            .unwrap(),
                        expected
                    );
                }
                assert_eq!(
                    crate::POOL_CREATIONS.with(|n| n.get()),
                    before,
                    "session assembly allocated another pool"
                );
                assert_eq!(session.thread_count(), threads);
            }
        }
    }
}

#[cfg(test)]
include!("mesh_attribution.rs");
