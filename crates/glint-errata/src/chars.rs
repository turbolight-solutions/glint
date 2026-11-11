//! Characters that can hide or reorder text (Trojan Source), smuggle invisible text (tag characters) or drive a
//! terminal: C0 controls, DEL, C1 controls, bidi marks, embeddings, overrides and isolates, and the invisible
//! default-ignorable characters (zero-width characters, fillers, variation selectors, interlinear annotation, tags).
//! None of them occurs in the errata feed of 2026-11-11.

fn hides_text(c: char) -> bool {
    matches!(
        c,
        '\u{7f}'..='\u{9f}'
            | '\u{34f}'
            | '\u{61c}'
            | '\u{115f}'..='\u{1160}'
            | '\u{17b4}'..='\u{17b5}'
            | '\u{180b}'..='\u{180f}'
            | '\u{200b}'..='\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{2069}'
            | '\u{3164}'
            | '\u{fe00}'..='\u{fe0f}'
            | '\u{feff}'
            | '\u{ffa0}'
            | '\u{fff9}'..='\u{fffb}'
            | '\u{e0000}'..='\u{e007f}'
            | '\u{e0100}'..='\u{e01ef}'
    )
}

/// A character errata feed text may not hold. Tab, line feed, carriage return and form feed are allowed: errata quote
/// paginated RFC text. So is the soft hyphen, which hides nothing and occurs in Verified errata (RFC 5878 erratum
/// 3513).
pub fn feed_forbidden(c: char) -> bool {
    matches!(c, '\u{0}'..='\u{8}' | '\u{b}' | '\u{e}'..='\u{1f}') || hides_text(c)
}

/// A character a single line we write (a ledger note, an added patch line) may not hold: everything
/// [`feed_forbidden`] refuses, every line break, and the soft hyphen. Only tab is allowed.
pub fn line_forbidden(c: char) -> bool {
    matches!(c, '\u{0}'..='\u{8}' | '\u{a}'..='\u{1f}' | '\u{ad}' | '\u{2028}' | '\u{2029}')
        || hides_text(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feed_text_keeps_page_layout_characters() {
        for c in ['\t', '\n', '\r', '\u{c}', '\u{ad}', 'a', 'é'] {
            assert!(!feed_forbidden(c), "{c:?}");
        }
        for c in [
            '\u{0}',
            '\u{b}',
            '\u{1b}',
            '\u{7f}',
            '\u{85}',
            '\u{61c}',
            '\u{180e}',
            '\u{200b}',
            '\u{200e}',
            '\u{200f}',
            '\u{202e}',
            '\u{2062}',
            '\u{2066}',
            '\u{feff}',
            '\u{34f}',
            '\u{3164}',
            '\u{fe0f}',
            '\u{fffa}',
            '\u{e0041}',
            '\u{e0100}',
        ] {
            assert!(feed_forbidden(c), "{c:?}");
        }
    }

    #[test]
    fn a_line_keeps_only_tab() {
        assert!(!line_forbidden('\t'));
        for c in [
            '\n', '\r', '\u{c}', '\u{ad}', '\u{2028}', '\u{2029}', '\u{202a}',
        ] {
            assert!(line_forbidden(c), "{c:?}");
        }
    }
}
