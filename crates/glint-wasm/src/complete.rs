//! Completion for the editor, from the concrete syntax tree of the block under the cursor and the corpus.
//!
//! Three contexts are told apart: the components of an object identifier value (the arcs the corpus knows below the
//! ones written so far), an `IMPORTS` clause (module names after `FROM`, the module's exported symbols before it),
//! and anywhere else in a module (its own and its imported names, and the built-in types).

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use glint_corpus::{CorpusProvider, Exports, Oid, preferred};
use glint_extract::{Block, BlockKind, Format, detect_format, extract};
use glint_sema::{
    DraftSource, Evaluator, ModuleModel, Options, SymbolKind, analyze, registered_arc,
};
use glint_syntax::{ParseOptions, SyntaxKind, SyntaxToken, ast, parse_source_with};
use serde::Serialize;

use crate::service::{FormatName, Service};

/// The most items one completion returns.
const MAX_ITEMS: usize = 400;

const BUILTIN_TYPES: &[&str] = &[
    "BIT STRING",
    "BMPString",
    "BOOLEAN",
    "CHOICE",
    "ENUMERATED",
    "GeneralizedTime",
    "IA5String",
    "INTEGER",
    "NULL",
    "OBJECT IDENTIFIER",
    "OCTET STRING",
    "PrintableString",
    "SEQUENCE",
    "SEQUENCE OF",
    "SET",
    "SET OF",
    "TeletexString",
    "UniversalString",
    "UTCTime",
    "UTF8String",
    "VisibleString",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Item {
    pub label: String,
    /// `type`, `value`, `class`, `module`, `arc`, `keyword`: the CodeMirror completion type.
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub detail: Option<String>,
    /// Text to insert when it differs from the label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub apply: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct Completion {
    /// Where the word being completed starts, in UTF-16 code units of the document.
    pub from: usize,
    pub items: Vec<Item>,
}

/// The byte offset of the UTF-16 position `position` in `text`.
pub fn utf16_to_byte(text: &str, position: usize) -> usize {
    let mut units = 0_usize;
    for (byte, c) in text.char_indices() {
        if units >= position {
            return byte;
        }
        units = units.saturating_add(c.len_utf16());
    }
    text.len()
}

/// The UTF-16 position of the byte offset `byte` in `text`.
pub fn byte_to_utf16(text: &str, byte: usize) -> usize {
    text.get(..byte)
        .map_or(0, |prefix| prefix.chars().map(char::len_utf16).sum())
}

/// The completions at UTF-16 position `position` of a document.
pub fn complete(
    service: &Service,
    text: &str,
    name: Option<&str>,
    format: Option<FormatName>,
    position: usize,
) -> Completion {
    let format = format.map_or_else(|| detect_format(name.map(Path::new), text), Format::from);
    let at = utf16_to_byte(text, position);
    let extraction = extract(text, format);
    let Some(block) = extraction
        .blocks
        .iter()
        .find(|b| b.origin.span.start <= at && at <= b.origin.span.end)
    else {
        return Completion::default();
    };
    let Some(cursor) = block_offset(block, at) else {
        return Completion::default();
    };
    let start = word_start(&block.text, cursor);
    let from = block
        .source_map
        .map_start(start)
        .map_or(position, |doc| byte_to_utf16(text, doc));
    // Assignments quoted in the prose belong to no module: the names they can use are those of the document's
    // modules.
    let context = if block.kind == BlockKind::Fragment {
        extraction
            .blocks
            .iter()
            .filter(|b| b.kind != BlockKind::Fragment)
            .flat_map(|b| module_models(&b.text))
            .collect()
    } else {
        Vec::new()
    };
    let items = complete_in_block(service, &block.text, start, &context);
    Completion { from, items }
}

/// The models of the modules in `text`.
fn module_models(text: &str) -> Vec<ModuleModel> {
    let parse = parse_source_with(text, ParseOptions::corpus());
    ast::SourceFile::cast(parse.syntax())
        .map(|file| {
            file.modules()
                .filter_map(|m| ModuleModel::build(&m))
                .collect()
        })
        .unwrap_or_default()
}

/// The offset in `block.text` that the document offset `at` falls on: the last one that maps at or before it.
fn block_offset(block: &Block, at: usize) -> Option<usize> {
    let (mut low, mut high) = (0_usize, block.text.len());
    if block.source_map.map_start(0).is_none_or(|first| first > at) {
        return None;
    }
    while low < high {
        let middle = low.saturating_add(high.saturating_sub(low).saturating_add(1) / 2);
        match block.source_map.map_start(middle) {
            Some(mapped) if mapped <= at => low = middle,
            _ => high = middle.saturating_sub(1),
        }
    }
    let mut offset = low;
    while !block.text.is_char_boundary(offset) {
        offset = offset.saturating_sub(1);
    }
    Some(offset)
}

/// The start of the name that ends at `cursor`.
fn word_start(text: &str, cursor: usize) -> usize {
    let before = text.get(..cursor).unwrap_or_default();
    let length: usize = before
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
        .map(char::len_utf8)
        .sum();
    cursor.saturating_sub(length)
}

fn significant(token: &SyntaxToken) -> bool {
    !token.kind().is_trivia()
}

fn complete_in_block(
    service: &Service,
    text: &str,
    start: usize,
    context: &[ModuleModel],
) -> Vec<Item> {
    let parse = parse_source_with(text, ParseOptions::corpus());
    // The assignments of a fragment, which has no module of its own.
    let fragment_names: Vec<String> = if context.is_empty() {
        Vec::new()
    } else {
        ast::SourceFile::cast(parse.syntax())
            .map(|file| {
                file.assignments()
                    .filter_map(|a| a.name().map(|n| n.text().to_owned()))
                    .collect()
            })
            .unwrap_or_default()
    };
    let tokens: Vec<SyntaxToken> = parse
        .syntax()
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(significant)
        .collect();
    let before: Vec<&SyntaxToken> = tokens
        .iter()
        .take_while(|t| usize::from(t.text_range().end()) <= start)
        .collect();
    let after: Vec<&SyntaxToken> = tokens
        .iter()
        .filter(|t| usize::from(t.text_range().start()) > start)
        .collect();

    let corpus = service.corpus();
    let options = Options {
        resolution_notes: false,
        ..Options::default()
    };
    let analysis = analyze(
        vec![DraftSource {
            text: text.to_owned(),
            parse,
        }],
        corpus,
        &options,
    );
    // A module still being written has no `END` yet, so its range stops at its last token, before the cursor: it
    // then reaches to the end of the block.
    let unterminated = |end: usize| {
        tokens
            .iter()
            .find(|t| usize::from(t.text_range().end()) == end)
            .is_none_or(|t| t.text() != "END")
    };
    let module = analysis.drafts.iter().copied().find(|&id| {
        analysis.graph.module(id).is_some_and(|m| {
            let range = m.model.range;
            let end = usize::from(range.end());
            usize::from(range.start()) <= start && (start <= end || unterminated(end))
        })
    });
    let model = module
        .and_then(|id| analysis.graph.module(id))
        .map(|m| &m.model);

    if let Some(components) = oid_components(&before) {
        let evaluator = Evaluator::new(&analysis.graph);
        let mut arcs: Vec<u128> = Vec::new();
        for (index, component) in components.iter().enumerate() {
            match component {
                OidPart::Number(n) => arcs.push(*n),
                OidPart::Name(name) => {
                    if let Some(arc) = registered_arc(&arcs, name) {
                        arcs.push(arc);
                    } else if index == 0
                        && let Some(oid) = module.and_then(|m| evaluator.oid_of_name(m, None, name))
                    {
                        arcs.extend_from_slice(oid.arcs());
                    } else {
                        return Vec::new();
                    }
                }
            }
        }
        return arc_items(service, &Oid::new(arcs), model, components.is_empty());
    }

    if let Some(clause_start) = imports_start(&before) {
        let previous = before.last().map(|t| t.text().to_owned());
        if previous.as_deref() == Some("FROM") {
            return module_items(service);
        }
        let in_clause = before.get(clause_start..).unwrap_or_default();
        let listed: HashSet<&str> = in_clause.iter().map(|t| t.text()).collect();
        if let Some(target) = next_from(&after) {
            return exported_items(service, &target, &listed);
        }
        return Vec::new();
    }

    reference_items(model, context, &fragment_names)
}

/// One component of an object identifier value written so far.
enum OidPart {
    Number(u128),
    Name(String),
}

/// The components before the cursor when it is inside the braces of an object identifier value: an `OBJECT IDENTIFIER`
/// value, a module identifier, or the identifier of a module in `IMPORTS`.
fn oid_components(before: &[&SyntaxToken]) -> Option<Vec<OidPart>> {
    let open = before
        .iter()
        .rposition(|t| matches!(t.kind(), SyntaxKind::LBrace | SyntaxKind::RBrace))?;
    let brace = before.get(open)?;
    if brace.kind() != SyntaxKind::LBrace {
        return None;
    }
    let ahead = |back: usize| {
        open.checked_sub(back)
            .and_then(|i| before.get(i))
            .map(|t| t.text())
    };
    let is_oid_value = ahead(1) == Some("::=") && ahead(2) == Some("IDENTIFIER");
    let is_module_identifier = ahead(1)
        .is_some_and(|t| t.starts_with(|c: char| c.is_ascii_uppercase()))
        && (ahead(2) == Some("FROM") || ahead(2).is_none() || ahead(2) == Some("END"));
    if !(is_oid_value || is_module_identifier) {
        return None;
    }
    let inside = before.get(open.saturating_add(1)..).unwrap_or_default();
    let mut parts = Vec::new();
    let mut i = 0_usize;
    while let Some(token) = inside.get(i) {
        match token.kind() {
            SyntaxKind::Number => parts.push(OidPart::Number(token.text().parse().ok()?)),
            SyntaxKind::LowerName => {
                let open_paren = inside.get(i.saturating_add(1));
                if open_paren.is_some_and(|t| t.kind() == SyntaxKind::LParen) {
                    let number = inside.get(i.saturating_add(2))?;
                    let close = inside.get(i.saturating_add(3))?;
                    if close.kind() != SyntaxKind::RParen {
                        return None;
                    }
                    parts.push(OidPart::Number(number.text().parse().ok()?));
                    i = i.saturating_add(3);
                } else {
                    parts.push(OidPart::Name(token.text().to_owned()));
                }
            }
            _ => return None,
        }
        i = i.saturating_add(1);
    }
    Some(parts)
}

/// The index in `before` just after the `IMPORTS` keyword when the cursor is in the clause.
fn imports_start(before: &[&SyntaxToken]) -> Option<usize> {
    let imports = before.iter().rposition(|t| t.text() == "IMPORTS")?;
    let closed = before
        .get(imports..)
        .unwrap_or_default()
        .iter()
        .any(|t| t.kind() == SyntaxKind::Semicolon);
    (!closed).then_some(imports.saturating_add(1))
}

/// The module name after the next `FROM`, if the clause has one after the cursor.
fn next_from(after: &[&SyntaxToken]) -> Option<String> {
    let mut iter = after.iter();
    for token in iter.by_ref() {
        if token.kind() == SyntaxKind::Semicolon {
            return None;
        }
        if token.text() == "FROM" {
            break;
        }
    }
    iter.next()
        .filter(|t| t.kind() == SyntaxKind::UpperName)
        .map(|t| t.text().to_owned())
}

/// `{ iso(1) identified-organization(3) dod(6) ... }`: an OID with the labels documents most often write.
fn labelled(service: &Service, oid: &Oid) -> String {
    let table = service.corpus().oid_labels();
    let mut parts = Vec::new();
    for length in 1..=oid.len() {
        let prefix = Oid::new(oid.arcs().get(..length).unwrap_or_default().to_vec());
        let arc = prefix.last().unwrap_or_default();
        let label = table
            .labels_of(&prefix)
            .iter()
            .max_by_key(|e| e.documents.len())
            .map(|e| e.label.clone());
        parts.push(match label {
            Some(label) => format!("{label}({arc})"),
            None => arc.to_string(),
        });
    }
    format!("{{ {} }}", parts.join(" "))
}

fn arc_items(
    service: &Service,
    prefix: &Oid,
    model: Option<&ModuleModel>,
    first: bool,
) -> Vec<Item> {
    let corpus = service.corpus();
    let mut by_arc: BTreeMap<u128, (String, usize, Option<String>)> = BTreeMap::new();
    if prefix.is_empty() {
        for (label, arc) in [("itu-t", 0), ("iso", 1), ("joint-iso-itu-t", 2)] {
            by_arc.insert(arc, (label.to_owned(), usize::MAX, None));
        }
    }
    for node in corpus.oid_labels().nodes() {
        if node.oid.parent().as_ref() != Some(prefix) {
            continue;
        }
        let (Some(arc), Some(best)) = (
            node.oid.last(),
            node.labels.iter().max_by_key(|e| e.documents.len()),
        ) else {
            continue;
        };
        let count = best.documents.len();
        if by_arc.get(&arc).is_none_or(|(_, c, _)| count > *c) {
            by_arc.insert(arc, (best.label.clone(), count, None));
        }
    }
    for assignment in corpus.assignments_under(prefix) {
        if assignment.oid.parent().as_ref() != Some(prefix) {
            continue;
        }
        if let Some(arc) = assignment.oid.last() {
            let entry = by_arc
                .entry(arc)
                .or_insert_with(|| (assignment.name.clone(), 0, None));
            entry.2.get_or_insert_with(|| {
                format!(
                    "{} in {}",
                    assignment.name,
                    glint_corpus::document_title(&assignment.document)
                )
            });
        }
    }
    let mut items: Vec<Item> = by_arc
        .into_iter()
        .map(|(arc, (label, _, detail))| Item {
            label: format!("{label}({arc})"),
            kind: "arc",
            detail: Some(detail.unwrap_or_else(|| prefix.child(arc).to_string())),
            apply: None,
        })
        .collect();
    if first && let Some(model) = model {
        for symbol in &model.symbols {
            if symbol.kind.is_value_like()
                && symbol.type_node.as_ref().is_some_and(|t| {
                    t.text().to_string().split_whitespace().collect::<Vec<_>>()
                        == ["OBJECT", "IDENTIFIER"]
                })
            {
                items.push(Item {
                    label: symbol.name.clone(),
                    kind: "value",
                    detail: Some("OBJECT IDENTIFIER".to_owned()),
                    apply: None,
                });
            }
        }
        for clause in &model.imports {
            for symbol in &clause.symbols {
                if symbol.name.starts_with(|c: char| c.is_ascii_lowercase()) {
                    items.push(Item {
                        label: symbol.name.clone(),
                        kind: "value",
                        detail: Some(format!("from {}", clause.module_name)),
                        apply: None,
                    });
                }
            }
        }
    }
    items.truncate(MAX_ITEMS);
    items
}

fn module_items(service: &Service) -> Vec<Item> {
    let corpus = service.corpus();
    let mut names: Vec<&str> = corpus.modules().iter().map(|m| m.name.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    names
        .into_iter()
        .filter_map(|name| {
            let index = preferred(corpus, corpus.by_name(name))?;
            let module = corpus.modules().get(index)?;
            let apply = module
                .oid
                .as_ref()
                .map(|oid| format!("{name} {}", labelled(service, oid)));
            Some(Item {
                label: name.to_owned(),
                kind: "module",
                detail: Some(module.source.title()),
                apply,
            })
        })
        .take(MAX_ITEMS)
        .collect()
}

fn exported_items(service: &Service, module_name: &str, listed: &HashSet<&str>) -> Vec<Item> {
    let corpus = service.corpus();
    let Some(record) =
        preferred(corpus, corpus.by_name(module_name)).and_then(|i| corpus.modules().get(i))
    else {
        return Vec::new();
    };
    let parse = parse_source_with(&record.text, ParseOptions::corpus());
    let model = ast::SourceFile::cast(parse.syntax()).and_then(|file| {
        file.modules()
            .find(|m| m.name().is_some_and(|n| n.text() == record.name))
            .and_then(|m| ModuleModel::build(&m))
    });
    let Some(model) = model else {
        return Vec::new();
    };
    let exported = |name: &str| match &record.exports {
        Exports::All => true,
        Exports::List(list) => list.iter().any(|n| n == name),
    };
    let detail = format!("{} ({})", record.name, record.source.title());
    model
        .symbols
        .iter()
        .filter(|s| exported(&s.name) && !listed.contains(s.name.as_str()))
        .map(|symbol| Item {
            label: if symbol.is_parameterized() {
                format!("{}{{}}", symbol.name)
            } else {
                symbol.name.clone()
            },
            kind: kind_name(symbol.kind),
            detail: Some(detail.clone()),
            apply: None,
        })
        .take(MAX_ITEMS)
        .collect()
}

fn kind_name(kind: SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Type | SymbolKind::SetOfValuesOrObjects => "type",
        SymbolKind::Class => "class",
        SymbolKind::Macro => "keyword",
        SymbolKind::Value | SymbolKind::ObjectOrValue | SymbolKind::Unknown => "value",
    }
}

fn case_kind(name: &str) -> &'static str {
    if name.starts_with(|c: char| c.is_ascii_lowercase()) {
        "value"
    } else {
        "type"
    }
}

/// The names usable at the cursor: those of `model`, the module it is in; or, in a fragment, its own and those of
/// the document's modules in `context`. And the built-in types.
fn reference_items(
    model: Option<&ModuleModel>,
    context: &[ModuleModel],
    fragment_names: &[String],
) -> Vec<Item> {
    // The built-in types first, so that a document with many names never loses them to the cap; the editor sorts.
    let mut items: Vec<Item> = BUILTIN_TYPES
        .iter()
        .map(|builtin| Item {
            label: (*builtin).to_owned(),
            kind: "keyword",
            detail: None,
            apply: None,
        })
        .collect();
    let mut seen: HashSet<String> = HashSet::new();
    for name in fragment_names {
        if seen.insert(name.clone()) {
            items.push(Item {
                label: name.clone(),
                kind: case_kind(name),
                detail: Some("this snippet".to_owned()),
                apply: None,
            });
        }
    }
    let models: Vec<(&ModuleModel, bool)> = match model {
        Some(model) => vec![(model, true)],
        None => context.iter().map(|m| (m, false)).collect(),
    };
    for (model, own) in models {
        for symbol in &model.symbols {
            if seen.insert(symbol.name.clone()) {
                items.push(Item {
                    label: symbol.name.clone(),
                    kind: kind_name(symbol.kind),
                    detail: Some(if own {
                        "this module".to_owned()
                    } else {
                        model.name.clone()
                    }),
                    apply: None,
                });
            }
        }
        for clause in &model.imports {
            for symbol in &clause.symbols {
                if seen.insert(symbol.name.clone()) {
                    items.push(Item {
                        label: symbol.name.clone(),
                        kind: case_kind(&symbol.name),
                        detail: Some(format!("from {}", clause.module_name)),
                        apply: None,
                    });
                }
            }
        }
    }
    items.truncate(MAX_ITEMS);
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_positions_round_trip() {
        let text = "a€b😀c";
        for (position, byte) in [(0, 0), (1, 1), (2, 4), (3, 5), (5, 9), (6, 10)] {
            assert_eq!(utf16_to_byte(text, position), byte);
            assert_eq!(byte_to_utf16(text, byte), position);
        }
    }
}
