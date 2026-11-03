/// Maps byte offsets to 1-based line and column numbers.
#[derive(Debug, Clone)]
pub struct LineIndex {
    line_starts: Vec<usize>,
    text: String,
}

impl LineIndex {
    /// Counts lines by the ASN.1 rule: LF, VT, FF and CR (CR LF counts once) end a line (X.680 12.1.6).
    pub fn new(text: &str) -> Self {
        Self::with_breaks(text, true)
    }

    /// Counts lines as editors do: only LF, CR LF and a lone CR end a line, so form feeds in text RFCs do not
    /// shift line numbers.
    pub fn for_document(text: &str) -> Self {
        Self::with_breaks(text, false)
    }

    fn with_breaks(text: &str, vertical_breaks: bool) -> Self {
        let mut line_starts = vec![0];
        let mut chars = text.char_indices().peekable();
        while let Some((offset, c)) = chars.next() {
            let ends_line = match c {
                '\n' => true,
                '\u{0B}' | '\u{0C}' => vertical_breaks,
                '\r' => !matches!(chars.peek(), Some((_, '\n'))),
                _ => false,
            };
            if ends_line {
                line_starts.push(offset.saturating_add(c.len_utf8()));
            }
        }
        Self {
            line_starts,
            text: text.to_owned(),
        }
    }

    /// 1-based line and column (counted in characters) of byte `offset`; offsets past the end clamp to the end.
    pub fn line_col(&self, offset: usize) -> (u32, u32) {
        let offset = offset.min(self.text.len());
        let line = self.line_starts.partition_point(|&start| start <= offset);
        let line_start = line
            .checked_sub(1)
            .and_then(|i| self.line_starts.get(i))
            .copied()
            .unwrap_or(0);
        let column = self
            .text
            .get(line_start..offset)
            .map_or(0, |prefix| prefix.chars().count());
        (
            saturating_u32(line),
            saturating_u32(column.saturating_add(1)),
        )
    }
}

fn saturating_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::LineIndex;

    #[test]
    fn line_and_column_are_one_based() {
        let index = LineIndex::new("ab\ncd\r\nef\rg");
        assert_eq!(index.line_col(0), (1, 1));
        assert_eq!(index.line_col(1), (1, 2));
        assert_eq!(index.line_col(3), (2, 1));
        assert_eq!(index.line_col(7), (3, 1));
        assert_eq!(index.line_col(10), (4, 1));
    }

    #[test]
    fn document_index_ignores_form_feeds() {
        let text = "a\n\u{0C}\nb";
        assert_eq!(LineIndex::new(text).line_col(4), (4, 1));
        assert_eq!(LineIndex::for_document(text).line_col(4), (3, 1));
    }

    #[test]
    fn columns_count_characters_not_bytes() {
        let index = LineIndex::new("é-x");
        assert_eq!(index.line_col(3), (1, 3));
    }

    #[test]
    fn offsets_past_the_end_clamp() {
        let index = LineIndex::new("a\nb");
        assert_eq!(index.line_col(99), (2, 2));
    }
}
