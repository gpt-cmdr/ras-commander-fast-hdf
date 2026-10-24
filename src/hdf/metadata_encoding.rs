use crate::UnsupportedCode;
use crate::{Error, ErrorKind, Result};
use crate::hdf5_reader::{io::Cursor, messages::datatype::parse_datatype_description};
#[derive(Debug)]
pub(super) struct Proof {
    pub width: usize,
    pub kind: Kind,
}
#[derive(Debug)]
pub(super) enum Kind {
    Integer {
        signed: bool,
        big: bool,
    },
    Float {
        big: bool,
    },
    /// HDF5 bitfield (class 4): full-precision, zero-offset, zero-padded bytes.
    Bitfield {
        big: bool,
    },
    Text {
        padding: u32,
        utf8: bool,
    },
    Compound(Vec<(String, usize, Proof)>),
    Unsupported(String),
}
fn corrupt(s: &str) -> Error {
    Error::new(ErrorKind::Corrupt, s)
}
fn le(b: &[u8], at: usize, n: usize) -> Result<u32> {
    let v = b
        .get(
            at..at
                .checked_add(n)
                .ok_or_else(|| corrupt("encoding offset overflow"))?,
        )
        .ok_or_else(|| corrupt("truncated datatype envelope"))?;
    Ok(v.iter().rev().fold(0, |a, x| (a << 8) | u32::from(*x)))
}
pub(super) fn proof(b: &[u8]) -> Result<Proof> {
    let mut cursor = Cursor::new(b);
    // This parser establishes consumed envelope length only. No discarded
    // numeric layout, flags or legacy member dimensions are trusted.
    parse_datatype_description(&mut cursor).map_err(super::map_error)?;
    let head = le(b, 0, 4)?;
    let version = (head >> 4) & 15;
    let flags = head >> 8;
    let width = le(b, 4, 4)? as usize;
    if !(1..=3).contains(&version) {
        return Ok(Proof {
            width,
            kind: Kind::Unsupported("datatype version".into()),
        });
    }
    let unsupported = |reason: &str| Kind::Unsupported(reason.into());
    let kind = match head & 15 {
        0 => {
            if width == 0
                || ![1, 2, 4, 8].contains(&width)
                || flags & !9 != 0
                || le(b, 8, 2)? != 0
                || le(b, 10, 2)? != (width * 8) as u32
            {
                unsupported("integer width/offset/precision/padding/sign/endian encoding")
            } else {
                Kind::Integer {
                    signed: flags & 8 != 0,
                    big: flags & 1 != 0,
                }
            }
        }
        1 => {
            let layout = match width {
                4 => Some((31, 23, 8, 23, 127)),
                8 => Some((63, 52, 11, 52, 1023)),
                _ => None,
            };
            if let Some((sign, exp, exp_size, mant_size, bias)) = layout {
                if flags & !0x0000_ff3f != 0
                    || flags & 0x3e != 0x20
                    || (flags >> 8) & 255 != sign
                    || le(b, 8, 2)? != 0
                    || le(b, 10, 2)? != (width * 8) as u32
                    || le(b, 12, 1)? != exp
                    || le(b, 13, 1)? != exp_size
                    || le(b, 14, 1)? != 0
                    || le(b, 15, 1)? != mant_size
                    || le(b, 16, 4)? != bias
                {
                    unsupported("non-IEEE floating layout/offset/padding/endian encoding")
                } else {
                    Kind::Float {
                        big: flags & 1 != 0,
                    }
                }
            } else {
                unsupported("floating width")
            }
        }
        4 => {
            // Same property layout as integers; only full-width bitfields with zero
            // padding are proven, so the stored bytes are the whole value.
            if ![1, 2, 4, 8].contains(&width)
                || flags & !1 != 0
                || le(b, 8, 2)? != 0
                || le(b, 10, 2)? != (width * 8) as u32
            {
                unsupported("bitfield width/offset/precision/padding encoding")
            } else {
                Kind::Bitfield {
                    big: flags & 1 != 0,
                }
            }
        }
        3 => {
            if width == 0 {
                unsupported("zero-width/variable string")
            } else if flags & !0xff != 0 || flags & 15 > 2 || (flags >> 4) & 15 > 1 {
                unsupported("reserved string padding/charset/flags")
            } else {
                Kind::Text {
                    padding: flags & 15,
                    utf8: flags & 0x10 != 0,
                }
            }
        }
        6 => {
            if width == 0 {
                return Err(corrupt("zero compound record width"));
            }
            if flags & !0xffff != 0 {
                return Ok(Proof {
                    width,
                    kind: unsupported("reserved compound flags"),
                });
            }
            let mut at = 8usize;
            let mut fields = Vec::new();
            let mut ranges = Vec::new();
            for _ in 0..flags & 0xffff {
                let tail = b
                    .get(at..)
                    .ok_or_else(|| corrupt("missing compound member"))?;
                let length = tail
                    .iter()
                    .position(|x| *x == 0)
                    .ok_or_else(|| corrupt("unterminated member name"))?;
                let name = std::str::from_utf8(&tail[..length])
                    .map_err(|_| corrupt("invalid member name UTF-8"))?
                    .to_owned();
                if fields.iter().any(|(n, _, _)| n == &name) {
                    return Err(corrupt("duplicate member name"));
                }
                at += if version < 3 {
                    (length + 1).next_multiple_of(8)
                } else {
                    length + 1
                };
                let n = if version < 3 {
                    4
                } else if width <= 255 {
                    1
                } else if width <= 65535 {
                    2
                } else if width <= 16777215 {
                    3
                } else {
                    4
                };
                let offset = le(b, at, n)? as usize;
                at += n;
                let mut extent_factor = 1usize;
                let dimensions = if version == 1 {
                    let dims = le(b, at, 1)?;
                    // Retain shape evidence: legacy scalar requires zero dimensionality.
                    b.get(at..at + 28)
                        .ok_or_else(|| corrupt("truncated legacy dimensions"))?;
                    if dims > 4 {
                        return Err(corrupt("legacy array dimensionality exceeds four"));
                    }
                    for i in 0..dims as usize {
                        extent_factor = extent_factor
                            .checked_mul(le(b, at + 12 + i * 4, 4)? as usize)
                            .ok_or_else(|| corrupt("legacy array extent overflow"))?;
                    }
                    // Historical zero-sized legacy arrays are refused as values;
                    // retain at least the base width for conservative overlap checks.
                    extent_factor = extent_factor.max(1);
                    at += 28;
                    dims
                } else {
                    0
                };
                let mut c = Cursor::new(b.get(at..).ok_or_else(|| corrupt("missing member type"))?);
                parse_datatype_description(&mut c).map_err(super::map_error)?;
                let used = c.position() as usize;
                let mut member = proof(
                    b.get(at..at + used)
                        .ok_or_else(|| corrupt("truncated member type"))?,
                )?;
                at += used;
                if dimensions != 0 {
                    member.kind = unsupported("legacy v1 member array dimensionality");
                }
                let extent = member
                    .width
                    .checked_mul(extent_factor)
                    .ok_or_else(|| corrupt("legacy array extent overflow"))?;
                let end = offset
                    .checked_add(extent)
                    .filter(|v| *v <= width)
                    .ok_or_else(|| corrupt("member outside record"))?;
                if member.width == 0 {
                    return Err(corrupt("zero member width"));
                }
                if ranges.iter().any(|&(a, z)| offset < z && a < end) {
                    return Err(corrupt("overlapping compound members"));
                }
                ranges.push((offset, end));
                fields.push((name, offset, member));
            }
            Kind::Compound(fields)
        }
        _ => unsupported("nested/array/vlen/reference metadata encoding"),
    };
    Ok(Proof { width, kind })
}
pub(super) fn decode(p: &Proof, raw: &[u8], count: u64) -> Result<super::MetadataValue> {
    use super::MetadataValue as V;
    if p.width == 0 {
        if let Kind::Unsupported(reason) = &p.kind {
            return Ok(V::unsupported(UnsupportedCode::Encoding, reason.clone()));
        }
    }
    let count = usize::try_from(count).map_err(|_| corrupt("metadata count exceeds usize"))?;
    if p.width == 0 || count.checked_mul(p.width) != Some(raw.len()) {
        return Err(corrupt("metadata shape/width/payload mismatch"));
    }
    if let Kind::Unsupported(reason) = &p.kind {
        return Ok(V::unsupported(UnsupportedCode::Encoding, reason.clone()));
    }
    let mut signed = Vec::new();
    let mut unsigned = Vec::new();
    let mut float = Vec::new();
    let mut text = Vec::new();
    if let Kind::Bitfield { .. } = p.kind {
        // Exact stored bytes, element width p.width in the declared order (h5py: uint<N>).
        return Ok(V::Bitfield(raw.to_vec()));
    }
    for b in raw.chunks_exact(p.width) {
        match p.kind {
            Kind::Integer { signed: sign, big } => {
                let u = if big {
                    b.iter().fold(0u64, |a, x| (a << 8) | u64::from(*x))
                } else {
                    b.iter().rev().fold(0u64, |a, x| (a << 8) | u64::from(*x))
                };
                if sign {
                    let shift = 64 - p.width * 8;
                    signed.push(((u << shift) as i64) >> shift);
                } else {
                    unsigned.push(u);
                }
            }
            Kind::Float { big } => {
                let u = if big {
                    b.iter().fold(0u64, |a, x| (a << 8) | u64::from(*x))
                } else {
                    b.iter().rev().fold(0u64, |a, x| (a << 8) | u64::from(*x))
                };
                float.push(if p.width == 4 {
                    f64::from(f32::from_bits(u as u32))
                } else {
                    f64::from_bits(u)
                });
            }
            Kind::Text { padding, utf8 } => {
                // h5py reads into NULLPAD memory and NumPy `S` drops trailing NULs:
                // NULLTERM keeps bytes before the first NUL; NULLPAD drops trailing NULs;
                // SPACEPAD drops trailing spaces and then trailing NULs (unwritten fill).
                let end = match padding {
                    0 => b.iter().position(|x| *x == 0).unwrap_or(b.len()),
                    1 => b.iter().rposition(|x| *x != 0).map_or(0, |i| i + 1),
                    _ => {
                        let spaces = b.iter().rposition(|x| *x != b' ').map_or(0, |i| i + 1);
                        b[..spaces]
                            .iter()
                            .rposition(|x| *x != 0)
                            .map_or(0, |i| i + 1)
                    }
                };
                let bytes = &b[..end];
                if !utf8 && !bytes.is_ascii() {
                    return Ok(V::unsupported(
                        UnsupportedCode::Encoding,
                        "non-ASCII byte in ASCII metadata; raw bytes retained",
                    ));
                }
                let Ok(value) = std::str::from_utf8(bytes) else {
                    return Ok(V::unsupported(
                        UnsupportedCode::Encoding,
                        "invalid UTF-8 metadata; raw bytes retained",
                    ));
                };
                text.push(value.to_owned());
            }
            _ => {
                return Ok(V::unsupported(
                    UnsupportedCode::Encoding,
                    "nested compound metadata",
                ))
            }
        }
    }
    Ok(match p.kind {
        Kind::Integer { signed: true, .. } => V::Signed(signed),
        Kind::Integer { .. } => V::Unsigned(unsigned),
        Kind::Float { .. } => V::Float(float),
        Kind::Text { .. } => V::Strings(text),
        _ => V::unsupported(UnsupportedCode::Encoding, "compound attribute"),
    })
}
#[cfg(test)]
mod bitfield_tests {
    use super::*;
    use crate::hdf::metadata::MetadataValue;
    /// Version-1 bitfield datatype envelope: class 4, flags, size, bit offset, precision.
    fn envelope(flags: u8, size: u32, offset: u16, precision: u16) -> Vec<u8> {
        let mut b = vec![0x14, flags, 0, 0];
        b.extend(size.to_le_bytes());
        b.extend(offset.to_le_bytes());
        b.extend(precision.to_le_bytes());
        b
    }
    #[test]
    fn full_width_bitfields_are_exact_bytes_and_partial_ones_refuse() {
        let p = proof(&envelope(0, 1, 0, 8)).unwrap();
        assert!(matches!(p.kind, Kind::Bitfield { big: false }));
        assert_eq!(
            decode(&p, &[1, 0, 255], 3).unwrap(),
            MetadataValue::Bitfield(vec![1, 0, 255])
        );
        assert!(decode(&p, &[1, 0], 3).is_err());
        let big = proof(&envelope(1, 2, 0, 16)).unwrap();
        assert!(matches!(big.kind, Kind::Bitfield { big: true }));
        assert_eq!(
            decode(&big, &[1, 2, 3, 4], 2).unwrap(),
            MetadataValue::Bitfield(vec![1, 2, 3, 4])
        );
        // Bit offset, partial precision, padding flags and odd widths stay unproven.
        for (flags, size, offset, precision) in [
            (0, 1, 1, 7),
            (0, 1, 0, 4),
            (2, 1, 0, 8),
            (4, 1, 0, 8),
            (0, 3, 0, 24),
        ] {
            let p = proof(&envelope(flags, size, offset, precision)).unwrap();
            assert!(
                matches!(p.kind, Kind::Unsupported(_)),
                "{flags} {size} {offset} {precision}"
            );
        }
    }
}
