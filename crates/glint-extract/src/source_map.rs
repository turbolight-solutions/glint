use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Segment {
    block: usize,
    block_len: usize,
    orig: usize,
    orig_len: usize,
}

impl Segment {
    fn block_end(&self) -> usize {
        self.block.saturating_add(self.block_len)
    }

    fn orig_end(&self) -> usize {
        self.orig.saturating_add(self.orig_len)
    }

    /// A segment whose two sides have the same length is copied byte for byte; any other is a single unit
    /// (an entity reference or a CR LF pair) that cannot be split.
    fn is_linear(&self) -> bool {
        self.block_len == self.orig_len
    }
}

/// Translates byte offsets in an extracted block back to byte offsets in the original document.
///
/// The map is piecewise: copied text maps one to one, and a unit that changed size on the way (such as `&lt;`
/// becoming `<`) maps as a whole.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SourceMap {
    segments: Vec<Segment>,
    len: usize,
}

impl SourceMap {
    /// A map for a block that is the original text itself.
    pub fn identity(len: usize) -> Self {
        let mut assembler = Assembler::default();
        assembler.push_segment_len(len, 0..len);
        assembler.finish_map()
    }

    /// Length in bytes of the block this map describes.
    pub fn block_len(&self) -> usize {
        self.len
    }

    /// The original offset of the character that starts at block `offset`.
    pub fn map_start(&self, offset: usize) -> Option<usize> {
        let offset = offset.min(self.len);
        let index = self.segments.partition_point(|s| s.block <= offset);
        match index.checked_sub(1).and_then(|i| self.segments.get(i)) {
            Some(segment) if offset < segment.block_end() => Some(if segment.is_linear() {
                segment
                    .orig
                    .saturating_add(offset.saturating_sub(segment.block))
            } else {
                segment.orig
            }),
            _ => self.segments.last().map(Segment::orig_end),
        }
    }

    /// The original offset just past the character that ends at block `offset`.
    pub fn map_end(&self, offset: usize) -> Option<usize> {
        let offset = offset.min(self.len);
        let index = self.segments.partition_point(|s| s.block < offset);
        let segment = index.checked_sub(1).and_then(|i| self.segments.get(i))?;
        Some(if segment.is_linear() {
            segment
                .orig
                .saturating_add(offset.saturating_sub(segment.block))
        } else {
            segment.orig_end()
        })
    }

    /// The original byte range covered by block `range`.
    pub fn map_range(&self, range: Range<usize>) -> Option<Range<usize>> {
        let start = self.map_start(range.start)?;
        if range.is_empty() {
            return Some(start..start);
        }
        let end = self.map_end(range.end)?;
        Some(start..end.max(start))
    }

    fn overlapping(&self, range: &Range<usize>) -> impl Iterator<Item = &Segment> {
        let first = self
            .segments
            .partition_point(|s| s.block_end() <= range.start);
        self.segments
            .iter()
            .skip(first)
            .take_while(move |s| s.block < range.end)
    }
}

/// A block's text together with its map, used as the source when building a smaller block from it.
#[derive(Debug, Clone, Copy)]
pub struct Mapped<'a> {
    pub text: &'a str,
    pub map: &'a SourceMap,
}

/// Builds a block text and its [`SourceMap`] piece by piece.
#[derive(Debug, Default)]
pub struct Assembler {
    text: String,
    segments: Vec<Segment>,
}

impl Assembler {
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Appends `text`, which stands for the original bytes `orig`.
    pub fn push_mapped(&mut self, text: &str, orig: Range<usize>) {
        if text.is_empty() {
            return;
        }
        self.text.push_str(text);
        self.push_segment_len(text.len(), orig);
    }

    /// Appends `text` copied verbatim from the original starting at `orig_start`.
    pub fn push_verbatim(&mut self, text: &str, orig_start: usize) {
        self.push_mapped(text, orig_start..orig_start.saturating_add(text.len()));
    }

    /// Appends `range` of an existing block, keeping its mapping to the original.
    pub fn push_slice(&mut self, source: Mapped<'_>, range: Range<usize>) {
        for segment in source.map.overlapping(&range) {
            let start = range.start.max(segment.block);
            let end = range.end.min(segment.block_end());
            let Some(piece) = source.text.get(start..end) else {
                continue;
            };
            if segment.is_linear() {
                self.push_verbatim(
                    piece,
                    segment
                        .orig
                        .saturating_add(start.saturating_sub(segment.block)),
                );
            } else {
                self.push_mapped(piece, segment.orig..segment.orig_end());
            }
        }
    }

    pub fn finish(self) -> (String, SourceMap) {
        let text = self.text;
        let len = text.len();
        (
            text,
            SourceMap {
                segments: self.segments,
                len,
            },
        )
    }

    fn finish_map(self) -> SourceMap {
        let len = self.segments.last().map_or(0, Segment::block_end);
        SourceMap {
            segments: self.segments,
            len,
        }
    }

    fn push_segment_len(&mut self, block_len: usize, orig: Range<usize>) {
        if block_len == 0 {
            return;
        }
        let block = self.segments.last().map_or(0, Segment::block_end);
        let segment = Segment {
            block,
            block_len,
            orig: orig.start,
            orig_len: orig.end.saturating_sub(orig.start),
        };
        if let Some(last) = self.segments.last_mut()
            && last.is_linear()
            && segment.is_linear()
            && last.orig_end() == segment.orig
        {
            last.block_len = last.block_len.saturating_add(segment.block_len);
            last.orig_len = last.orig_len.saturating_add(segment.orig_len);
            return;
        }
        self.segments.push(segment);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_maps_offsets_to_themselves() {
        let map = SourceMap::identity(10);
        assert_eq!(map.map_start(0), Some(0));
        assert_eq!(map.map_start(7), Some(7));
        assert_eq!(map.map_range(2..5), Some(2..5));
        assert_eq!(map.map_start(10), Some(10));
    }

    #[test]
    fn entity_maps_as_a_unit() {
        let mut assembler = Assembler::default();
        assembler.push_verbatim("a", 10);
        assembler.push_mapped("<", 11..15);
        assembler.push_verbatim("b", 15);
        let (text, map) = assembler.finish();
        assert_eq!(text, "a<b");
        assert_eq!(map.map_range(1..2), Some(11..15));
        assert_eq!(map.map_start(2), Some(15));
        assert_eq!(map.map_range(0..3), Some(10..16));
    }

    #[test]
    fn adjacent_verbatim_pieces_merge() {
        let mut assembler = Assembler::default();
        assembler.push_verbatim("ab", 0);
        assembler.push_verbatim("cd", 2);
        assert_eq!(assembler.segments.len(), 1);
    }

    #[test]
    fn slices_keep_the_mapping() {
        let mut assembler = Assembler::default();
        assembler.push_verbatim("xx", 5);
        assembler.push_mapped("<", 7..11);
        assembler.push_verbatim("yy", 11);
        let (text, map) = assembler.finish();

        let mut rebuilt = Assembler::default();
        rebuilt.push_slice(
            Mapped {
                text: &text,
                map: &map,
            },
            1..4,
        );
        let (new_text, new_map) = rebuilt.finish();
        assert_eq!(new_text, "x<y");
        assert_eq!(new_map.map_range(0..1), Some(6..7));
        assert_eq!(new_map.map_range(1..2), Some(7..11));
        assert_eq!(new_map.map_range(2..3), Some(11..12));
    }

    #[test]
    fn empty_map_maps_nothing() {
        let (_, map) = Assembler::default().finish();
        assert_eq!(map.map_start(0), None);
        assert_eq!(map.map_range(0..0), None);
    }
}
