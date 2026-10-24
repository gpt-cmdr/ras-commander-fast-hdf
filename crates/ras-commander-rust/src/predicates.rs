//! Exact geometric predicates for ring validation and winding.
//!
//! `orient2d` is Shewchuk's adaptive-precision predicate from the `robust` crate, mapped to
//! three orientations exactly as `geo` 0.29's `RobustKernel` maps it. `segment_contact`
//! classifies two segments exactly as `geo` 0.29's `line_intersection` does (no contact, one
//! point, or a collinear overlap), without computing the intersection coordinates, which
//! ring validation never uses.
//!
//! The engine used `geo` 0.29 for these two functions only. Depending on `geo` tied every
//! consumer to its `i_overlay`/`i_float` versions, which conflict with `geo` 0.33 (used by
//! terrio 0.5 and RustGIS). The verdicts are unchanged: the unit tests compare both functions
//! with `geo` 0.29 (a dev-dependency) on exhaustive small-integer and random inputs.
//!
//! Port of `geo` 0.29.3 `algorithm/line_intersection.rs`, `kernels/robust.rs` and the
//! closed-interval `Rect` intersection tests (MIT OR Apache-2.0, the GeoRust contributors).

pub(crate) type Point = [f64; 2];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Orientation {
    CounterClockwise,
    Clockwise,
    Collinear,
}

/// Orientation of `r` relative to the directed line `p` -> `q`.
pub(crate) fn orient2d(p: Point, q: Point, r: Point) -> Orientation {
    let c = |a: Point| robust::Coord { x: a[0], y: a[1] };
    let o = robust::orient2d(c(p), c(q), c(r));
    if o < 0. {
        Orientation::Clockwise
    } else if o > 0. {
        Orientation::CounterClockwise
    } else {
        Orientation::Collinear
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Segment {
    pub start: Point,
    pub end: Point,
}

impl Segment {
    pub fn new(start: Point, end: Point) -> Self {
        Self { start, end }
    }
    /// The closed bounding box as (min, max).
    fn bounds(&self) -> (Point, Point) {
        (
            [
                self.start[0].min(self.end[0]),
                self.start[1].min(self.end[1]),
            ],
            [
                self.start[0].max(self.end[0]),
                self.start[1].max(self.end[1]),
            ],
        )
    }
}

/// How two segments meet, in `geo` 0.29 `line_intersection` terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Contact {
    /// One point (`LineIntersection::SinglePoint`, proper or not).
    Point,
    /// A collinear overlap of positive length (`LineIntersection::Collinear`).
    Collinear,
}

fn boxes_intersect(a: (Point, Point), b: (Point, Point)) -> bool {
    !(a.1[0] < b.0[0] || a.1[1] < b.0[1] || a.0[0] > b.1[0] || a.0[1] > b.1[1])
}

fn box_contains(b: (Point, Point), p: Point) -> bool {
    p[0] >= b.0[0] && p[1] >= b.0[1] && p[0] <= b.1[0] && p[1] <= b.1[1]
}

/// `None` when the segments do not meet.
pub(crate) fn segment_contact(p: Segment, q: Segment) -> Option<Contact> {
    use Orientation::*;
    let (pb, qb) = (p.bounds(), q.bounds());
    if !boxes_intersect(pb, qb) {
        return None;
    }
    let p_q1 = orient2d(p.start, p.end, q.start);
    let p_q2 = orient2d(p.start, p.end, q.end);
    if matches!(
        (p_q1, p_q2),
        (Clockwise, Clockwise) | (CounterClockwise, CounterClockwise)
    ) {
        return None;
    }
    let q_p1 = orient2d(q.start, q.end, p.start);
    let q_p2 = orient2d(q.start, q.end, p.end);
    if matches!(
        (q_p1, q_p2),
        (Clockwise, Clockwise) | (CounterClockwise, CounterClockwise)
    ) {
        return None;
    }
    if (p_q1, p_q2, q_p1, q_p2) == (Collinear, Collinear, Collinear, Collinear) {
        // geo's `collinear_intersection`: a shared endpoint alone is a single point.
        return match (
            box_contains(pb, q.start),
            box_contains(pb, q.end),
            box_contains(qb, p.start),
            box_contains(qb, p.end),
        ) {
            (true, true, _, _) | (_, _, true, true) => Some(Contact::Collinear),
            (true, false, true, false) if q.start == p.start => Some(Contact::Point),
            (true, _, true, _) => Some(Contact::Collinear),
            (true, false, false, true) if q.start == p.end => Some(Contact::Point),
            (true, _, _, true) => Some(Contact::Collinear),
            (false, true, true, false) if q.end == p.start => Some(Contact::Point),
            (_, true, true, _) => Some(Contact::Collinear),
            (false, true, false, true) if q.end == p.end => Some(Contact::Point),
            (_, true, _, true) => Some(Contact::Collinear),
            _ => None,
        };
    }
    // Not collinear and not separated: one intersection point (endpoint or proper).
    Some(Contact::Point)
}

#[cfg(test)]
mod tests {
    use super::*;
    use geo::kernels::{Kernel, Orientation as GeoOrientation, RobustKernel};
    use geo::line_intersection::{line_intersection, LineIntersection};

    fn geo_orient(p: Point, q: Point, r: Point) -> Orientation {
        match RobustKernel::orient2d(p.into(), q.into(), r.into()) {
            GeoOrientation::Clockwise => Orientation::Clockwise,
            GeoOrientation::CounterClockwise => Orientation::CounterClockwise,
            GeoOrientation::Collinear => Orientation::Collinear,
        }
    }

    fn geo_contact(p: Segment, q: Segment) -> Option<Contact> {
        line_intersection(
            geo::Line::new(p.start, p.end),
            geo::Line::new(q.start, q.end),
        )
        .map(|i| match i {
            LineIntersection::SinglePoint { .. } => Contact::Point,
            LineIntersection::Collinear { .. } => Contact::Collinear,
        })
    }

    fn check(p: Segment, q: Segment) {
        assert_eq!(segment_contact(p, q), geo_contact(p, q), "{p:?} {q:?}");
        assert_eq!(
            orient2d(p.start, p.end, q.start),
            geo_orient(p.start, p.end, q.start)
        );
    }

    /// Every pair of segments on a 4 x 4 integer grid, including degenerate (zero-length) ones
    /// and every collinear, touching and overlapping arrangement.
    #[test]
    fn matches_geo_0_29_on_every_small_grid_pair() {
        let pts: Vec<Point> = (0..4)
            .flat_map(|x| (0..4).map(move |y| [x as f64, y as f64]))
            .collect();
        let segs: Vec<Segment> = pts
            .iter()
            .flat_map(|&a| pts.iter().map(move |&b| Segment::new(a, b)))
            .collect();
        let mut n = 0;
        for &p in &segs {
            for &q in &segs {
                check(p, q);
                n += 1;
            }
        }
        assert_eq!(n, 256 * 256);
    }

    /// Random and nearly collinear segments at projected-coordinate magnitudes, where adaptive
    /// precision decides the sign.
    #[test]
    fn matches_geo_0_29_on_random_and_near_degenerate_pairs() {
        let mut state = 0x9E37_79B9_7F4A_7C15_u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / (1u64 << 53) as f64
        };
        for _ in 0..200_000 {
            let base = [2_000_000.0 + next() * 10.0, 320_000.0 + next() * 10.0];
            let a = [base[0] + next(), base[1] + next()];
            let b = [base[0] + next(), base[1] + next()];
            let t = next();
            // A point on (or within an ulp or two of) segment a-b.
            let on = [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])];
            let c = [base[0] + next(), base[1] + next()];
            check(Segment::new(a, b), Segment::new(on, c));
            check(Segment::new(a, b), Segment::new(c, on));
            check(Segment::new(a, on), Segment::new(on, b));
            check(Segment::new(a, b), Segment::new(b, c));
        }
    }
}
