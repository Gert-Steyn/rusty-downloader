//! Byte-range segments and IDM-style dynamic splitting.
//!
//! A download starts as one segment covering the whole file. Whenever a
//! connection slot is free, the segment with the most bytes left is split in
//! half and the upper half gets its own connection. The running worker sees its
//! segment's `end` shrink and stops there.

/// An inclusive byte range `[start, end]` with `written` bytes done from `start`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    pub start: u64,
    pub end: u64,
    pub written: u64,
}

// An inclusive range always holds at least one byte, so `is_empty` would be meaningless.
#[expect(clippy::len_without_is_empty)]
impl Segment {
    pub fn new(start: u64, end: u64) -> Self {
        debug_assert!(end >= start);
        Self {
            start,
            end,
            written: 0,
        }
    }

    pub fn len(&self) -> u64 {
        self.end - self.start + 1
    }

    pub fn remaining(&self) -> u64 {
        self.len().saturating_sub(self.written)
    }

    /// Offset of the next byte to fetch.
    pub fn next_offset(&self) -> u64 {
        self.start + self.written
    }

    pub fn is_done(&self) -> bool {
        self.remaining() == 0
    }

    /// Record `bytes` more written, never past the (possibly shrunk) end.
    pub fn advance(&mut self, bytes: u64) {
        self.written = (self.written + bytes).min(self.len());
    }
}

/// Split the segment with the most remaining bytes, appending the new upper
/// half. Returns its index, or `None` if no segment has at least
/// `2 * min_size` bytes left.
pub fn split_largest(segments: &mut Vec<Segment>, min_size: u64) -> Option<usize> {
    let (idx, remaining) = segments
        .iter()
        .enumerate()
        .map(|(i, s)| (i, s.remaining()))
        .max_by_key(|&(_, remaining)| remaining)?;
    if remaining < min_size.max(1).saturating_mul(2) {
        return None;
    }

    let segment = &mut segments[idx];
    let mid = segment.next_offset() + remaining / 2;
    let upper = Segment::new(mid, segment.end);
    segment.end = mid - 1;
    segments.push(upper);
    Some(segments.len() - 1)
}

pub fn total_written(segments: &[Segment]) -> u64 {
    segments.iter().map(|s| s.written).sum()
}

/// True if the segments tile `[0, size)` exactly, so they can be resumed.
pub fn covers(segments: &[Segment], size: u64) -> bool {
    let mut sorted: Vec<_> = segments.iter().collect();
    sorted.sort_by_key(|s| s.start);
    let mut next = 0;
    for s in sorted {
        if s.start != next || s.end < s.start || s.written > s.len() {
            return false;
        }
        next = s.end + 1;
    }
    next == size && size > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: u64 = 1024 * 1024;

    #[test]
    fn segment_accounting() {
        let mut s = Segment::new(100, 199);
        assert_eq!((s.len(), s.remaining(), s.next_offset()), (100, 100, 100));
        s.advance(40);
        assert_eq!((s.remaining(), s.next_offset()), (60, 140));
        s.advance(1_000);
        assert!(s.is_done());
        assert_eq!(s.written, 100);
    }

    #[test]
    fn splitting_halves_the_remaining_part() {
        let mut segs = vec![Segment::new(0, 8 * MIB - 1)];
        segs[0].advance(2 * MIB);

        let new = split_largest(&mut segs, MIB).unwrap();

        assert_eq!(new, 1);
        assert_eq!(
            segs[0],
            Segment {
                start: 0,
                end: 5 * MIB - 1,
                written: 2 * MIB
            }
        );
        assert_eq!(segs[1], Segment::new(5 * MIB, 8 * MIB - 1));
        assert!(covers(&segs, 8 * MIB));
    }

    #[test]
    fn splitting_picks_the_largest_remaining() {
        let mut segs = vec![
            Segment::new(0, 4 * MIB - 1),
            Segment::new(4 * MIB, 16 * MIB - 1),
        ];
        split_largest(&mut segs, MIB).unwrap();
        assert_eq!(segs[1].end, 10 * MIB - 1);
        assert_eq!(segs[2], Segment::new(10 * MIB, 16 * MIB - 1));
    }

    #[test]
    fn no_split_below_twice_the_minimum() {
        let mut segs = vec![Segment::new(0, 2 * MIB - 2)];
        assert_eq!(split_largest(&mut segs, MIB), None);
        assert_eq!(segs.len(), 1);
    }

    #[test]
    fn repeated_splits_keep_full_coverage() {
        let size = 10 * MIB + 12_345;
        let mut segs = vec![Segment::new(0, size - 1)];
        while split_largest(&mut segs, MIB).is_some() {}
        assert!(covers(&segs, size));
        assert!(segs.iter().all(|s| s.len() >= MIB));
    }

    #[test]
    fn coverage_rejects_gaps_overlaps_and_wrong_size() {
        assert!(!covers(&[Segment::new(0, 9), Segment::new(11, 19)], 20));
        assert!(!covers(&[Segment::new(0, 10), Segment::new(10, 19)], 20));
        assert!(!covers(&[Segment::new(0, 9)], 20));
        assert!(!covers(&[], 0));
    }
}
