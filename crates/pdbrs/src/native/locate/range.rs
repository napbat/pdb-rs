//! A sorted index of address ranges that can overlap.

use alloc::vec::Vec;

use super::push;
use crate::Result;

/// One range of RVAs with a value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Span<T> {
    /// The first RVA.
    pub(super) start: u32,
    /// The RVA after the last byte. It can be `2^32`.
    pub(super) end: u64,
    /// The value of the range.
    pub(super) value: T,
}

/// Ranges of RVAs sorted by start, with a running maximum of the ends.
///
/// The ranges can overlap, for example the procedures that identical code
/// folding gives the same address. The running maximum bounds the backward
/// search for the ranges that hold an address.
#[derive(Clone, Debug)]
pub(super) struct RangeIndex<T> {
    /// The ranges, sorted by start. Equal starts keep the insertion order.
    spans: Vec<Span<T>>,
    /// The largest end of the ranges at and before each position.
    max_end: Vec<u64>,
}

impl<T: Copy> RangeIndex<T> {
    /// Sort `spans` and compute the running maximum of the ends.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ErrorKind::LimitExceeded`] if the maximum list cannot
    /// be allocated.
    pub(super) fn new(mut spans: Vec<Span<T>>) -> Result<Self> {
        spans.sort_by_key(|span| span.start);
        let mut max_end = Vec::new();
        let mut largest = 0;
        for span in &spans {
            largest = largest.max(span.end);
            push(
                &mut max_end,
                largest,
                "address range index cannot be allocated",
            )?;
        }
        Ok(Self { spans, max_end })
    }

    /// Return the ranges that hold `rva`, from the largest start down.
    ///
    /// Ranges with the same start come in reverse insertion order.
    pub(super) fn containing(&self, rva: u32) -> impl Iterator<Item = &Span<T>> + '_ {
        let after = self.spans.partition_point(|span| span.start <= rva);
        let address = u64::from(rva);
        self.spans[..after]
            .iter()
            .zip(&self.max_end[..after])
            .rev()
            .take_while(move |&(_, &end)| end > address)
            .filter_map(move |(span, _)| (span.end > address).then_some(span))
    }

    /// Return the range that holds `rva` and has the largest start.
    ///
    /// For ranges with the same start, return the first one in insertion order.
    pub(super) fn find(&self, rva: u32) -> Option<&Span<T>> {
        self.find_first_preferred(rva, |_| false)
    }

    /// Return the range that holds `rva` and has the largest start.
    ///
    /// For ranges with the same start, return the last one in insertion
    /// order for which `prefer` is `true`, or else the last one.
    pub(super) fn find_last_preferred<F>(&self, rva: u32, prefer: F) -> Option<&Span<T>>
    where
        F: Fn(&T) -> bool,
    {
        let mut spans = self.containing(rva);
        // Ranges with the same start come in reverse insertion order.
        let last = spans.next()?;
        if prefer(&last.value) {
            return Some(last);
        }
        let preferred = spans
            .take_while(|span| span.start == last.start)
            .find(|span| prefer(&span.value));
        Some(preferred.unwrap_or(last))
    }

    /// Return the range that holds `rva` and has the largest start.
    ///
    /// For ranges with the same start, return the first one in insertion
    /// order for which `prefer` is `true`, or else the first one.
    pub(super) fn find_first_preferred<F>(&self, rva: u32, prefer: F) -> Option<&Span<T>>
    where
        F: Fn(&T) -> bool,
    {
        let mut first: Option<&Span<T>> = None;
        let mut preferred: Option<&Span<T>> = None;
        for span in self.containing(rva) {
            if first.is_some_and(|found| found.start != span.start) {
                break;
            }
            // Ranges with the same start come in reverse insertion order.
            first = Some(span);
            if prefer(&span.value) {
                preferred = Some(span);
            }
        }
        preferred.or(first)
    }

    /// Return the range with the largest start at or below `rva`, also if
    /// it does not hold `rva`.
    pub(super) fn preceding(&self, rva: u32) -> Option<&Span<T>> {
        let after = self.spans.partition_point(|span| span.start <= rva);
        let start = self.spans[..after].last()?.start;
        let first = self.spans[..after].partition_point(|span| span.start < start);
        self.spans.get(first)
    }

    /// Return the ranges in start order.
    pub(super) fn spans(&self) -> &[Span<T>] {
        &self.spans
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use super::{RangeIndex, Span};

    fn span(start: u32, length: u64, value: u8) -> Span<u8> {
        Span {
            start,
            end: u64::from(start) + length,
            value,
        }
    }

    fn values(index: &RangeIndex<u8>, rva: u32) -> Vec<u8> {
        index.containing(rva).map(|span| span.value).collect()
    }

    #[test]
    fn finds_ranges_at_their_boundaries_and_not_in_gaps() {
        let index = RangeIndex::new(vec![span(0x2000, 0x10, 2), span(0x1000, 0x20, 1)]).unwrap();
        assert_eq!(index.find(0x1000).map(|span| span.value), Some(1));
        assert_eq!(index.find(0x101f).map(|span| span.value), Some(1));
        assert_eq!(index.find(0x1020), None);
        assert_eq!(index.find(0x1fff), None);
        assert_eq!(index.find(0x0fff), None);
        assert_eq!(index.find(0x2000).map(|span| span.value), Some(2));
        assert_eq!(index.find(0x2010), None);
        assert_eq!(index.preceding(0x1fff).map(|span| span.value), Some(1));
        assert_eq!(index.preceding(0x0fff), None);
    }

    #[test]
    fn finds_an_outer_range_behind_shorter_ranges() {
        let index = RangeIndex::new(vec![
            span(0x1000, 0x100, 1),
            span(0x1010, 0x10, 2),
            span(0x1030, 0x10, 3),
        ])
        .unwrap();
        assert_eq!(values(&index, 0x1035), vec![3, 1]);
        assert_eq!(values(&index, 0x1025), vec![1]);
        assert_eq!(index.find(0x1015).map(|span| span.value), Some(2));
    }

    #[test]
    fn keeps_the_first_of_ranges_with_the_same_start() {
        let index = RangeIndex::new(vec![
            span(0x1000, 0x10, 1),
            span(0x1000, 0x20, 2),
            span(0x1000, 0x10, 3),
        ])
        .unwrap();
        assert_eq!(index.find(0x1008).map(|span| span.value), Some(1));
        assert_eq!(index.find(0x1018).map(|span| span.value), Some(2));
        assert_eq!(index.preceding(0x2000).map(|span| span.value), Some(1));
    }

    #[test]
    fn prefers_the_last_matching_range_with_the_same_start() {
        let index = RangeIndex::new(vec![
            span(0x1000, 0x10, 1),
            span(0x1000, 0x10, 2),
            span(0x1000, 0x10, 3),
            span(0x0800, 0x1000, 4),
        ])
        .unwrap();
        let last = |rva| index.find_last_preferred(rva, |&value| value < 3);
        assert_eq!(last(0x1008).map(|span| span.value), Some(2));
        assert_eq!(
            index
                .find_last_preferred(0x1008, |_| false)
                .map(|span| span.value),
            Some(3)
        );
        assert_eq!(last(0x1010).map(|span| span.value), Some(4));
        assert_eq!(last(0x1800), None);
    }

    #[test]
    fn empty_ranges_and_the_end_of_the_address_space() {
        let index = RangeIndex::new(vec![span(0x1000, 0, 1), span(u32::MAX, 1, 2)]).unwrap();
        assert_eq!(index.find(0x1000), None);
        assert_eq!(index.find(u32::MAX).map(|span| span.value), Some(2));
    }
}
