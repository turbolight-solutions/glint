/// One line of a document: byte offsets of its start, of the end of its content and of its end including the break.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Line {
    pub start: usize,
    pub content_end: usize,
    pub end: usize,
}

impl Line {
    pub fn content<'a>(&self, text: &'a str) -> &'a str {
        text.get(self.start..self.content_end).unwrap_or_default()
    }

    pub fn terminator<'a>(&self, text: &'a str) -> &'a str {
        text.get(self.content_end..self.end).unwrap_or_default()
    }
}

/// Splits `text` at LF, CR LF and lone CR.
pub(crate) fn split_lines(text: &str) -> Vec<Line> {
    let bytes = text.as_bytes();
    let mut lines = Vec::new();
    let mut start = 0;
    let mut index = 0;
    while index < bytes.len() {
        let (content_end, end) = match bytes.get(index) {
            Some(b'\n') => (index, index.saturating_add(1)),
            Some(b'\r') if bytes.get(index.saturating_add(1)) == Some(&b'\n') => {
                (index, index.saturating_add(2))
            }
            Some(b'\r') => (index, index.saturating_add(1)),
            _ => {
                index = index.saturating_add(1);
                continue;
            }
        };
        lines.push(Line {
            start,
            content_end,
            end,
        });
        start = end;
        index = end;
    }
    if start < bytes.len() {
        lines.push(Line {
            start,
            content_end: bytes.len(),
            end: bytes.len(),
        });
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_all_line_breaks() {
        let text = "a\nb\r\nc\rd";
        let lines = split_lines(text);
        let contents: Vec<_> = lines.iter().map(|l| l.content(text)).collect();
        assert_eq!(contents, ["a", "b", "c", "d"]);
        assert_eq!(lines.get(1).map(|l| l.terminator(text)), Some("\r\n"));
        assert_eq!(lines.get(3).map(|l| l.terminator(text)), Some(""));
    }

    #[test]
    fn empty_text_has_no_lines() {
        assert!(split_lines("").is_empty());
    }
}
