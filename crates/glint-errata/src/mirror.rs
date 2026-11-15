//! Mirroring an XML erratum patch into the RFC's published text.
//!
//! An RFC from 8650 on is published as RFCXML (`rfcN.xml`, which Glint extracts from) and as `xml2rfc`'s rendering of
//! it (`rfcN.txt`, which a reader sees). `xml2rfc` copies the content of `<sourcecode>` and `<artwork>` into the text
//! verbatim, entities decoded, adding only a uniform left indentation (and dropping blank lines around the content and
//! whitespace at line ends). So when an XML patch changes only such content, the same change can be made to the text
//! block by block, without a renderer: the k-th element of the XML is the k-th block of the text, found in order, each
//! after the one before it. Matching by block rather than by text keeps an edit on the right copy of a definition an
//! RFC repeats (a module and its inline copy).
//!
//! An XML patch that changes anything else (prose, a title, the number of elements) cannot be mirrored: the text side
//! must be edited by hand.

use similar::{Algorithm, DiffTag, capture_diff_slices};

use glint_extract::{
    CodeElement, TextBlock as Block, block_margin, code_elements, display_lines, is_rendered,
    locate_blocks,
};

use crate::patch::{Source, SourceFormat, make_patch, split_lines};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MirrorError {
    /// The XML patch changes something outside the content of `<sourcecode>` and `<artwork>` elements, or adds or
    /// removes such an element.
    #[error("{0}")]
    Outside(String),
    /// A block the mirroring needs is not found in the text, or not found unambiguously.
    #[error("{0}")]
    Unlocated(String),
    /// Anything else: the XML change shows nothing in the text, or no patch can be made.
    #[error("{0}")]
    Other(String),
}

/// The 1-based line of byte `offset` in `text`.
fn line_of(text: &str, offset: usize) -> usize {
    text.get(..offset)
        .map_or(0, |before| before.matches('\n').count())
        .saturating_add(1)
}

fn describe(xml: &str, k: usize, element: &CodeElement) -> String {
    format!(
        "<{}> element {} (XML line {})",
        element.element,
        k.saturating_add(1),
        line_of(xml, element.content.start)
    )
}

/// The places in `text[from..to]` where `lines` is a block.
fn blocks_in(text: &[&str], lines: &[String], from: usize, to: usize) -> Vec<Block> {
    (from..to.min(text.len()))
        .filter(|&at| at.saturating_add(lines.len()) <= to)
        .filter_map(|at| {
            block_margin(text, at, lines).map(|margin| Block {
                start: at,
                len: lines.len(),
                indent: margin.indent,
                bar: margin.bar,
                breaks: Vec::new(),
            })
        })
        .collect()
}

/// Each element's block in the text, in order ([`locate_blocks`]).
fn locate(
    xml: &str,
    elements: &[CodeElement],
    text: &[&str],
) -> Result<Vec<Option<Block>>, MirrorError> {
    locate_blocks(elements, text).map_err(|k| {
        let what = elements.get(k).map_or_else(
            || format!("element {}", k.saturating_add(1)),
            |e| describe(xml, k, e),
        );
        MirrorError::Unlocated(format!(
            "{what} is not found in the text after the block before it"
        ))
    })
}

/// The XML around the elements' content: the text before the first, between each two, and after the last.
fn skeleton<'a>(xml: &'a str, elements: &[CodeElement]) -> Vec<(usize, &'a str)> {
    let mut pieces = Vec::with_capacity(elements.len().saturating_add(1));
    let mut at = 0usize;
    for element in elements {
        pieces.push((at, xml.get(at..element.content.start).unwrap_or_default()));
        at = element.content.end;
    }
    pieces.push((at, xml.get(at..).unwrap_or_default()));
    pieces
}

/// Fails unless `before` and `after` differ only inside the content of their `<sourcecode>` and `<artwork>` elements.
fn only_content_changes(
    before: &str,
    old: &[CodeElement],
    after: &str,
    new: &[CodeElement],
) -> Result<(), MirrorError> {
    if old.len() != new.len() {
        return Err(MirrorError::Outside(format!(
            "the XML patch changes the number of <sourcecode> and <artwork> elements ({} to {})",
            old.len(),
            new.len()
        )));
    }
    for ((at, a), (_, b)) in skeleton(before, old).into_iter().zip(skeleton(after, new)) {
        if a != b {
            let common = a
                .char_indices()
                .zip(b.chars())
                .find(|((_, x), y)| x != y)
                .map_or(a.len().min(b.len()), |((i, _), _)| i);
            return Err(MirrorError::Outside(format!(
                "the XML patch changes the XML outside <sourcecode> and <artwork> content (XML line {})",
                line_of(before, at.saturating_add(common))
            )));
        }
    }
    Ok(())
}

/// The new lines of a block: `block`'s lines of the text with the change from `old` to `new` (display lines) made,
/// lines the change keeps copied from the text, and new lines indented like the block.
fn changed_block(text: &[&str], block: &Block, old: &[String], new: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(new.len());
    for op in capture_diff_slices(Algorithm::Myers, old, new) {
        let (tag, old_range, new_range) = op.as_tag_tuple();
        if tag == DiffTag::Equal {
            let from = block.start.saturating_add(old_range.start);
            let to = block.start.saturating_add(old_range.end);
            out.extend(
                text.get(from..to)
                    .unwrap_or_default()
                    .iter()
                    .map(|l| (*l).to_owned()),
            );
        } else {
            out.extend(new.get(new_range).unwrap_or_default().iter().map(|line| {
                let margin = block.margin();
                if line.is_empty() {
                    format!("{}\n", margin.blank())
                } else {
                    format!("{}{line}\n", margin.text())
                }
            }));
        }
    }
    out
}

/// `text` (the published text plus the RFC's lower text patches) with the change from `xml_before` to `xml_after`
/// (the source plus the lower XML patches, then plus this erratum's) made to the same blocks.
pub fn mirror_text(xml_before: &str, xml_after: &str, text: &str) -> Result<String, MirrorError> {
    let (old, new) = (code_elements(xml_before), code_elements(xml_after));
    only_content_changes(xml_before, &old, xml_after, &new)?;
    let changed: Vec<usize> = old
        .iter()
        .zip(&new)
        .enumerate()
        .filter(|(_, (a, b))| display_lines(&a.text) != display_lines(&b.text))
        .map(|(k, _)| k)
        .collect();
    if changed.is_empty() {
        return Err(MirrorError::Other(
            "the XML patch changes nothing the text shows (only whitespace at line ends or blank lines around a block)"
                .to_owned(),
        ));
    }
    for &k in &changed {
        if let Some(element) = old.get(k)
            && (!is_rendered(element) || display_lines(&element.text).is_empty())
        {
            return Err(MirrorError::Unlocated(format!(
                "{} changes, but the text does not show it",
                describe(xml_before, k, element)
            )));
        }
    }
    let lines = split_lines(text);
    let blocks = locate(xml_before, &old, &lines)?;
    let mut out = String::with_capacity(text.len());
    let mut at = 0usize;
    for &k in &changed {
        let (Some(Some(block)), Some(a), Some(b)) = (blocks.get(k), old.get(k), new.get(k)) else {
            continue;
        };
        // RFCs from 8650 on, the only ones read from RFCXML, are not paginated.
        if !block.breaks.is_empty() {
            return Err(MirrorError::Unlocated(format!(
                "{} runs across a page break of the text",
                describe(xml_before, k, a)
            )));
        }
        // The block must be the only one with this content between its neighbours, or the k-th block was found in
        // the wrong place.
        let from = blocks
            .get(..k)
            .unwrap_or_default()
            .iter()
            .flatten()
            .next_back()
            .map_or(0, Block::end);
        let to = blocks
            .get(k.saturating_add(1)..)
            .unwrap_or_default()
            .iter()
            .flatten()
            .next()
            .map_or(lines.len(), |next| next.start);
        let (old_lines, new_lines) = (display_lines(&a.text), display_lines(&b.text));
        let places = blocks_in(&lines, &old_lines, from, to).len();
        if places != 1 {
            return Err(MirrorError::Unlocated(format!(
                "{} is found {places} times between the blocks around it",
                describe(xml_before, k, a)
            )));
        }
        out.extend(
            lines
                .get(at..block.start)
                .unwrap_or_default()
                .iter()
                .copied(),
        );
        out.extend(changed_block(&lines, block, &old_lines, &new_lines));
        at = block.end();
    }
    out.extend(lines.get(at..).unwrap_or_default().iter().copied());
    Ok(out)
}

/// The text patch of RFC `rfc` mirroring the change from `xml_before` to `xml_after`: a patch against `text` (the
/// published text plus the RFC's lower text patches) that [`crate::apply_patch`] accepts.
pub fn mirror_patch(
    rfc: u32,
    xml_before: &str,
    xml_after: &str,
    text: &str,
) -> Result<String, MirrorError> {
    let mirrored = mirror_text(xml_before, xml_after, text)?;
    let source = Source {
        rfc,
        format: SourceFormat::Text,
    };
    make_patch(source, text, &mirrored).map_err(|e| MirrorError::Other(e.0))
}

/// Whether a text patch (taking `text_before` to `text_after`) agrees with an XML patch (taking `xml_before` to
/// `xml_after`) where they overlap. When the XML change can be mirrored, the text change must be exactly the
/// mirrored one. When the XML change reaches outside the blocks (the text was edited by hand), every block of the
/// corrected XML must be in the corrected text, in order, provided the uncorrected blocks were in the uncorrected
/// text. When a block cannot be found in the text at all, nothing can be compared.
pub fn agree(
    xml_before: &str,
    xml_after: &str,
    text_before: &str,
    text_after: &str,
) -> Result<(), String> {
    match mirror_text(xml_before, xml_after, text_before) {
        Ok(mirrored) if mirrored == text_after => Ok(()),
        Ok(_) => Err(
            "the text patch does not make the change the XML patch makes to the <sourcecode> and <artwork> blocks"
                .to_owned(),
        ),
        Err(MirrorError::Outside(_)) => {
            let old = code_elements(xml_before);
            if locate(xml_before, &old, &split_lines(text_before)).is_err() {
                return Ok(());
            }
            let new = code_elements(xml_after);
            locate(xml_after, &new, &split_lines(text_after))
                .map(|_| ())
                .map_err(|e| format!("the corrected text does not hold the corrected XML's blocks: {e}"))
        }
        Err(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::apply_patch;

    const TEXT: Source = Source {
        rfc: 100,
        format: SourceFormat::Text,
    };

    fn xml(body: &str) -> String {
        format!(
            "<rfc>\n<middle>\n<section>\n<name>One</name>\n{body}</section>\n</middle>\n</rfc>\n"
        )
    }

    fn mirror(before: &str, after: &str, text: &str) -> Result<String, MirrorError> {
        let patch = mirror_patch(100, before, after, text)?;
        assert!(
            patch.starts_with("--- a/rfc100.txt\n+++ b/rfc100.txt\n"),
            "{patch}"
        );
        let applied = apply_patch(text, &patch, TEXT).map_err(|e| MirrorError::Other(e.0))?;
        assert_eq!(Ok(applied.clone()), mirror_text(before, after, text));
        Ok(applied)
    }

    #[test]
    fn a_change_in_a_quoted_block_keeps_the_bar() {
        // xml2rfc draws a <sourcecode> inside a <blockquote> behind a bar (RFC 8951 section 4).
        let before = xml(
            "<blockquote>\n<t>follows:</t>\n<sourcecode type=\"asn.1\">\nA ::= SEQUENCE {\n  b  INTEGER }\n\nC ::= BOOLEAN\n</sourcecode>\n</blockquote>\n",
        );
        let after = before.replace("  b  INTEGER }", "  b  INTEGER,\n\n  c  BOOLEAN }");
        let text = "1.  One\n\n   |  follows:\n   |  \n   |     A ::= SEQUENCE {\n   |       b  INTEGER }\n   |  \n   |     C ::= BOOLEAN\n\nEnd\n";
        assert_eq!(
            mirror(&before, &after, text),
            Ok(text.replace(
                "   |       b  INTEGER }\n",
                "   |       b  INTEGER,\n   |\n   |       c  BOOLEAN }\n"
            ))
        );
    }

    #[test]
    fn the_change_lands_with_the_blocks_indentation() {
        let before = xml(
            "<t>Prose.</t>\n<sourcecode type=\"asn.1\">\n   A ::= SEQUENCE {\n      a  INTEGER \n   }\n</sourcecode>\n",
        );
        let after = before.replace("a  INTEGER ", "a  INTEGER (0..9)");
        let text =
            "1.  One\n\n   Prose.\n\n      A ::= SEQUENCE {\n         a  INTEGER\n      }\n\nEnd\n";
        assert_eq!(
            mirror(&before, &after, text),
            Ok(text.replace("a  INTEGER\n", "a  INTEGER (0..9)\n"))
        );
    }

    #[test]
    fn entities_are_decoded_and_cdata_unwrapped() {
        let before = xml(
            "<sourcecode type=\"asn.1\">\nA ::= INTEGER (0..9) -- a &lt; 10 &amp; b\n</sourcecode>\n\
             <artwork><![CDATA[\nB ::= BOOLEAN -- <b>\n]]></artwork>\n",
        );
        let after = before
            .replace("a &lt; 10 &amp; b", "a &lt; 11 &amp; b")
            .replace("-- <b>", "-- <c>");
        let text = "   A ::= INTEGER (0..9) -- a < 10 & b\n\n   B ::= BOOLEAN -- <b>\n";
        assert_eq!(
            mirror(&before, &after, text),
            Ok("   A ::= INTEGER (0..9) -- a < 11 & b\n\n   B ::= BOOLEAN -- <c>\n".to_owned())
        );
    }

    #[test]
    fn a_repeated_definition_changes_in_its_own_block_only() {
        // Block 1 is an inline copy of the type that block 2's module defines: matching by text alone would find the
        // first copy.
        let before = xml(
            "<sourcecode type=\"asn.1\">\nX ::= INTEGER\n</sourcecode>\n<t>More.</t>\n\
             <sourcecode type=\"asn.1\">\nM DEFINITIONS ::= BEGIN\nX ::= INTEGER\nEND\n</sourcecode>\n",
        );
        let after = before.replace("BEGIN\nX ::= INTEGER", "BEGIN\nX ::= INTEGER (0..9)");
        let text = "   X ::= INTEGER\n\n   More.\n\n   M DEFINITIONS ::= BEGIN\n   X ::= INTEGER\n   END\n";
        assert_eq!(
            mirror(&before, &after, text),
            Ok(text.replace(
                "BEGIN\n   X ::= INTEGER\n",
                "BEGIN\n   X ::= INTEGER (0..9)\n"
            ))
        );
        // And the other way round: the inline copy changes, the module does not.
        let after = before.replacen("X ::= INTEGER", "X ::= INTEGER (0..9)", 1);
        assert_eq!(
            mirror(&before, &after, text),
            Ok(text.replacen("X ::= INTEGER", "X ::= INTEGER (0..9)", 1))
        );
    }

    #[test]
    fn the_first_line_may_sit_on_the_tag_line() {
        let before = xml("<sourcecode type=\"asn.1\">A ::= INTEGER\nB ::= BOOLEAN</sourcecode>\n");
        let after = before.replace("B ::= BOOLEAN", "B ::= NULL\nC ::= REAL");
        let text = "1.  One\n\n   A ::= INTEGER\n   B ::= BOOLEAN\n";
        assert_eq!(
            mirror(&before, &after, text),
            Ok("1.  One\n\n   A ::= INTEGER\n   B ::= NULL\n   C ::= REAL\n".to_owned())
        );
    }

    #[test]
    fn blank_lines_and_removed_lines() {
        let before = xml("<sourcecode>\n\nA ::= INTEGER\n\nB ::= BOOLEAN\n\n</sourcecode>\n");
        let after = before.replace("\nB ::= BOOLEAN\n", "");
        let text = "   A ::= INTEGER\n\n   B ::= BOOLEAN\n\nAfter\n";
        assert_eq!(
            mirror(&before, &after, text),
            Ok("   A ::= INTEGER\n\nAfter\n".to_owned())
        );
    }

    #[test]
    fn a_change_outside_the_blocks_must_be_made_by_hand() {
        let before = xml("<t>Prose.</t>\n<sourcecode>\nA ::= INTEGER\n</sourcecode>\n");
        let text = "   Prose.\n\n   A ::= INTEGER\n";
        let prose = before
            .replace("Prose.", "Better prose.")
            .replace("A ::= INTEGER", "A ::= BOOLEAN");
        assert!(matches!(
            mirror_text(&before, &prose, text),
            Err(MirrorError::Outside(m)) if m.contains("XML line 5")
        ));
        let attribute = before.replace("<sourcecode>", "<sourcecode type=\"asn.1\">");
        assert!(matches!(
            mirror_text(&before, &attribute, text),
            Err(MirrorError::Outside(_))
        ));
        let added = before.replace("</sourcecode>\n", "</sourcecode>\n<artwork>x</artwork>\n");
        assert!(matches!(
            mirror_text(&before, &added, text),
            Err(MirrorError::Outside(m)) if m.contains("number of")
        ));
    }

    #[test]
    fn a_block_the_text_lacks_is_an_error() {
        let before = xml(
            "<sourcecode>\nA ::= INTEGER\n</sourcecode>\n<sourcecode>\nB ::= BOOLEAN\n</sourcecode>\n",
        );
        let after = before.replace("B ::= BOOLEAN", "B ::= NULL");
        // The blocks out of order: block 2 is before block 1.
        let text = "   B ::= BOOLEAN\n\n   A ::= INTEGER\n";
        assert!(matches!(
            mirror_text(&before, &after, text),
            Err(MirrorError::Unlocated(m)) if m.contains("<sourcecode> element 2 (XML line 8)")
        ));
        // Lines indented unevenly are not the block.
        let before = before.replace("B ::= BOOLEAN", "B ::= SEQUENCE {\n}");
        let after = before.replace("{\n}", "{\n  b NULL\n}");
        let text = "   A ::= INTEGER\n\n   B ::= SEQUENCE {\n  }\n";
        assert!(matches!(
            mirror_text(&before, &after, text),
            Err(MirrorError::Unlocated(_))
        ));
        assert!(mirror_text(&before, &after, &text.replace("\n  }", "\n   }")).is_ok());
    }

    #[test]
    fn a_block_across_a_page_break_is_not_mirrored() {
        let before = xml("<sourcecode>\nA ::= SEQUENCE {\n  b  INTEGER }\n</sourcecode>\n");
        let after = before.replace("b  INTEGER", "b  BOOLEAN");
        let text = "   A ::= SEQUENCE {\n\nAuthor                    Standards Track                  [Page 3]\n\u{c}\nRFC 100                        One                       October 2026\n\n     b  INTEGER }\n";
        assert!(matches!(
            mirror_text(&before, &after, text),
            Err(MirrorError::Unlocated(m)) if m.contains("runs across a page break")
        ));
    }

    #[test]
    fn a_block_found_twice_between_its_neighbours_is_ambiguous() {
        let before = xml(
            "<sourcecode>\nA ::= INTEGER\n</sourcecode>\n<artwork type=\"svg\"><svg/></artwork>\n",
        );
        let after = before.replace("A ::= INTEGER", "A ::= NULL");
        let text = "   A ::= INTEGER\n\n   A ::= INTEGER\n";
        assert!(matches!(
            mirror_text(&before, &after, text),
            Err(MirrorError::Unlocated(m)) if m.contains("found 2 times")
        ));
    }

    #[test]
    fn whitespace_at_line_ends_shows_nothing() {
        let before = xml("<sourcecode>\nA ::= INTEGER  \n</sourcecode>\n");
        let after = before.replace("INTEGER  ", "INTEGER");
        assert!(matches!(
            mirror_text(&before, &after, "   A ::= INTEGER\n"),
            Err(MirrorError::Other(_))
        ));
    }

    #[test]
    fn agreement_of_a_text_patch_with_the_xml_patch() {
        let before = xml("<t>Prose.</t>\n<sourcecode>\nA ::= INTEGER\n</sourcecode>\n");
        let after = before.replace("A ::= INTEGER", "A ::= BOOLEAN");
        let text = "   Prose.\n\n   A ::= INTEGER\n";
        let good = text.replace("INTEGER", "BOOLEAN");
        assert_eq!(agree(&before, &after, text, &good), Ok(()));
        assert!(agree(&before, &after, text, &text.replace("INTEGER", "NULL")).is_err());
        // By hand: the prose changes too, and the corrected block must be in the corrected text.
        let both = after.replace("Prose.", "Prose, corrected.");
        let by_hand = good.replace("Prose.", "Prose, corrected.");
        assert_eq!(agree(&before, &both, text, &by_hand), Ok(()));
        assert!(
            agree(
                &before,
                &both,
                text,
                &text.replace("Prose.", "Prose, corrected.")
            )
            .is_err_and(|e| e.contains("element 1"))
        );
    }
}
