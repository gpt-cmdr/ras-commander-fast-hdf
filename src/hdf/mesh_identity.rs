//! Exact parent/child mesh comparison in deterministic field and stored-row order.
use super::{
    mesh_geometry::{budget, invalid, selected, MeshGeometryField},
    Identity, NumericValues, ReadOnlyHdfSession, Slice,
};
use crate::{Error, ErrorKind, Result, UnsupportedCode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MeshMismatchKind {
    Shape,
    NumericKind,
    Value,
}
impl MeshMismatchKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Shape => "shape",
            Self::NumericKind => "numeric_kind",
            Self::Value => "value",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct MeshMismatch {
    pub field: MeshGeometryField,
    pub kind: MeshMismatchKind,
    /// Stored row/column for a value mismatch; absent for shape/type differences.
    pub element: Option<(u64, u64)>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct MeshIdentityComparison {
    pub left: Identity,
    pub right: Identity,
    /// None certifies exact equality of all three required arrays, including NaN bits.
    pub first_mismatch: Option<MeshMismatch>,
}
fn first_difference(a: &NumericValues, b: &NumericValues) -> Option<usize> {
    macro_rules! eq {
        ($a:expr, $b:expr) => {
            $a.iter().zip($b).position(|(a, b)| a != b)
        };
    }
    match (a, b) {
        (NumericValues::I8(a), NumericValues::I8(b)) => eq!(a, b),
        (NumericValues::I16(a), NumericValues::I16(b)) => eq!(a, b),
        (NumericValues::I32(a), NumericValues::I32(b)) => eq!(a, b),
        (NumericValues::I64(a), NumericValues::I64(b)) => eq!(a, b),
        (NumericValues::U8(a), NumericValues::U8(b)) => eq!(a, b),
        (NumericValues::U16(a), NumericValues::U16(b)) => eq!(a, b),
        (NumericValues::U32(a), NumericValues::U32(b)) => eq!(a, b),
        (NumericValues::U64(a), NumericValues::U64(b)) => eq!(a, b),
        (NumericValues::F32(a), NumericValues::F32(b)) => a
            .iter()
            .zip(b)
            .position(|(a, b)| a.to_bits() != b.to_bits()),
        (NumericValues::F64(a), NumericValues::F64(b)) => a
            .iter()
            .zip(b)
            .position(|(a, b)| a.to_bits() != b.to_bits()),
        _ => Some(0),
    }
}
impl ReadOnlyHdfSession {
    /// Compare centers, face-cell indexes and normals/length in that order.
    /// Each input chunk is bounded by max_bytes; at most two such chunks are retained.
    /// Unknown, absent or damaged arrays are errors, never an equality result.
    pub fn compare_mesh_identity(
        &self,
        other: &ReadOnlyHdfSession,
        area: &str,
        metadata_max_bytes: usize,
        max_bytes: usize,
    ) -> Result<MeshIdentityComparison> {
        let left = self.mesh_geometry(area, metadata_max_bytes)?;
        let right = other.mesh_geometry(area, metadata_max_bytes)?;
        let mut result = MeshIdentityComparison {
            left: left.mesh.id,
            right: right.mesh.id,
            first_mismatch: None,
        };
        for field in [
            MeshGeometryField::CellCenters,
            MeshGeometryField::FaceCellIndexes,
            MeshGeometryField::FaceNormalsAndLength,
        ] {
            let a = left
                .fields
                .iter()
                .find(|d| d.field == field)
                .ok_or_else(|| invalid("unknown field"))?;
            let b = right
                .fields
                .iter()
                .find(|d| d.field == field)
                .ok_or_else(|| invalid("unknown field"))?;
            for d in [a, b] {
                match &d.capability {
                    crate::Capability::Supported => {}
                    crate::Capability::Unsupported { code, reason } => {
                        return Err(Error::unsupported(*code, reason))
                    }
                    crate::Capability::Unavailable { reason } => {
                        return Err(Error::new(ErrorKind::NotFound, reason))
                    }
                    _ => {
                        return Err(Error::new(
                            ErrorKind::Corrupt,
                            d.capability.reason().unwrap_or("invalid mesh"),
                        ))
                    }
                }
            }
            let mismatch = if a.shape != b.shape {
                Some(MeshMismatchKind::Shape)
            } else if a.kind != b.kind {
                Some(MeshMismatchKind::NumericKind)
            } else {
                None
            };
            if let Some(kind) = mismatch {
                result.first_mismatch = Some(MeshMismatch {
                    field,
                    kind,
                    element: None,
                });
                return Ok(result);
            }
            let width = match a.kind {
                Some(super::NumericKind::I8 | super::NumericKind::U8) => 1,
                Some(super::NumericKind::I16 | super::NumericKind::U16) => 2,
                Some(
                    super::NumericKind::I32 | super::NumericKind::U32 | super::NumericKind::F32,
                ) => 4,
                _ => 8,
            };
            let row_bytes = usize::try_from(a.shape[1])
                .ok()
                .and_then(|n| n.checked_mul(width))
                .ok_or_else(|| invalid("mesh row size overflow"))?;
            budget(row_bytes, max_bytes)?;
            let chunk_rows = u64::try_from(max_bytes / row_bytes)
                .map_err(|_| invalid("chunk rows exceed u64"))?
                .min(65536);
            if chunk_rows == 0 {
                return Err(invalid("one mesh row exceeds budget")
                    .with_code(UnsupportedCode::ResourceLimit));
            }
            let mut start = 0;
            while start < a.shape[0] {
                let end = start.saturating_add(chunk_rows).min(a.shape[0]);
                let av = selected(self, &a.path, Some(Slice::new(start, end, 1)), max_bytes)?;
                let bv = selected(other, &b.path, Some(Slice::new(start, end, 1)), max_bytes)?;
                if let Some(i) = first_difference(&av.values, &bv.values) {
                    let i = i as u64;
                    result.first_mismatch = Some(MeshMismatch {
                        field,
                        kind: MeshMismatchKind::Value,
                        element: Some((start + i / a.shape[1], i % a.shape[1])),
                    });
                    return Ok(result);
                }
                start = end;
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn float_comparison_is_bit_exact() {
        assert_eq!(
            first_difference(
                &NumericValues::F32(vec![f32::from_bits(0x7fc00001)]),
                &NumericValues::F32(vec![f32::from_bits(0x7fc00001)])
            ),
            None
        );
        assert_eq!(
            first_difference(
                &NumericValues::F32(vec![0.0]),
                &NumericValues::F32(vec![-0.0])
            ),
            Some(0)
        );
        assert_eq!(
            first_difference(
                &NumericValues::F64(vec![f64::from_bits(0x7ff8000000000001)]),
                &NumericValues::F64(vec![f64::from_bits(0x7ff8000000000002)])
            ),
            Some(0)
        );
    }
}
