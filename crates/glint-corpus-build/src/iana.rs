//! Reads IANA's SMI Numbers registry (`smi-numbers.xml`) for the registries of ASN.1 module identifiers: the
//! authority on which module an identifier under the PKIX, S/MIME, LTANS and similar arcs belongs to.

use std::collections::HashMap;
use std::path::Path;

use glint_corpus::{IanaModuleId, Oid};
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

/// Whether a registry assigns module identifiers, judged by its title (`SMI Security for PKIX Module Identifier`,
/// `... msec module-identifers`, `SMI Security for Mechanism ASN.1 Modules`).
fn is_module_registry(title: &str) -> bool {
    let title = title.to_ascii_lowercase();
    ["module identif", "module-identif", "asn.1 module"]
        .iter()
        .any(|marker| title.contains(marker))
}

/// The arc a registry assigns under: the last parenthesized dotted OID of its title or description, or the OID in its
/// id (`smi-numbers-1.3.6.1.5.5.7.0`).
fn registry_arc(id: &str, title: &str, description: &str) -> Option<Oid> {
    let parenthesized = |text: &str| -> Option<Oid> {
        text.rmatch_indices('(').find_map(|(at, _)| {
            let rest = text.get(at.saturating_add(1)..)?;
            let inner = rest.get(..rest.find(')')?)?;
            inner.contains('.').then(|| inner.trim().parse().ok())?
        })
    };
    parenthesized(title)
        .or_else(|| parenthesized(description))
        .or_else(|| {
            id.strip_prefix("smi-numbers-")?
                .parse()
                .ok()
                .filter(|o: &Oid| o.len() > 1)
        })
}

#[derive(Default)]
struct Registry {
    id: String,
    title: String,
    description: String,
    entries: Vec<Entry>,
}

#[derive(Default)]
struct Entry {
    value: String,
    description: String,
    name: String,
    documents: Vec<String>,
    errata: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Title,
    Description,
    Value,
    Name,
    Other,
}

fn attribute(start: &BytesStart<'_>, name: &str) -> Option<String> {
    start
        .attributes()
        .with_checks(false)
        .filter_map(Result::ok)
        .find(|attr| attr.key.local_name().as_ref() == name)
        .map(|attr| attr.value.to_string())
}

fn entity(name: &str) -> Option<char> {
    Some(match name {
        "lt" => '<',
        "gt" => '>',
        "amp" => '&',
        "apos" => '\'',
        "quot" => '"',
        _ => {
            let digits = name.strip_prefix('#')?;
            let code = match digits.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => digits.parse().ok()?,
            };
            char::from_u32(code)?
        }
    })
}

/// Reads the module identifier registries of `smi-numbers.xml`. `errata` maps erratum ids to the document they are
/// reported against, so that an entry assigned through an erratum names its document.
pub fn read_iana(
    path: &Path,
    errata: &HashMap<String, String>,
) -> Result<Vec<IanaModuleId>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let registries = parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut out = Vec::new();
    for registry in registries {
        if !is_module_registry(&registry.title) {
            continue;
        }
        let Some(arc) = registry_arc(&registry.id, &registry.title, &registry.description) else {
            return Err(format!(
                "{}: no arc found for the registry `{}`",
                path.display(),
                registry.id
            ));
        };
        for entry in registry.entries {
            let value = entry.value.trim();
            let oid = if value.contains('.') {
                match value.parse::<Oid>() {
                    Ok(oid) if arc.is_prefix_of(&oid) => oid,
                    _ => continue,
                }
            } else {
                match value.parse::<u128>() {
                    Ok(number) => arc.child(number),
                    Err(_) => continue, // a range of unassigned values
                }
            };
            let description = if entry.description.trim().is_empty() {
                entry.name.trim().to_owned()
            } else {
                entry.description.trim().to_owned()
            };
            let lower = description.to_ascii_lowercase();
            if description.is_empty()
                || lower.starts_with("reserved")
                || lower.starts_with("unassigned")
            {
                continue;
            }
            let mut documents = entry.documents;
            for erratum in &entry.errata {
                if let Some(document) = errata.get(erratum) {
                    documents.push(document.clone());
                }
            }
            documents.dedup();
            out.push(IanaModuleId {
                oid,
                description,
                registry: registry.title.trim().to_owned(),
                documents,
                errata: entry.errata,
            });
        }
    }
    out.sort_by(|a, b| a.oid.cmp(&b.oid));
    out.dedup_by(|a, b| a.oid == b.oid);
    Ok(out)
}

fn parse(text: &str) -> Result<Vec<Registry>, quick_xml::Error> {
    let mut reader = Reader::from_str(text);
    // Registries nest; a record belongs to the innermost one.
    let mut open: Vec<Registry> = Vec::new();
    let mut done: Vec<Registry> = Vec::new();
    let mut entry: Option<Entry> = None;
    let mut field = Field::Other;
    let mut depth_in_field = 0_u32;
    loop {
        let event = reader.read_event()?;
        let push_text = |chunk: &str, open: &mut Vec<Registry>, entry: &mut Option<Entry>| {
            let target = match (field, entry.as_mut(), open.last_mut()) {
                (Field::Value, Some(e), _) => &mut e.value,
                (Field::Description, Some(e), _) => &mut e.description,
                (Field::Name, Some(e), _) => &mut e.name,
                (Field::Title, None, Some(r)) => &mut r.title,
                (Field::Description, None, Some(r)) => &mut r.description,
                _ => return,
            };
            target.push_str(chunk);
        };
        match event {
            Event::Start(start) => {
                let name = start.local_name().as_ref().to_owned();
                if field != Field::Other {
                    depth_in_field = depth_in_field.saturating_add(1);
                    continue;
                }
                match name.as_str() {
                    "registry" => open.push(Registry {
                        id: attribute(&start, "id").unwrap_or_default(),
                        ..Registry::default()
                    }),
                    "record" => entry = Some(Entry::default()),
                    "title" => field = Field::Title,
                    "description" => field = Field::Description,
                    "value" => field = Field::Value,
                    "name" => field = Field::Name,
                    _ => {}
                }
            }
            Event::Empty(start) => {
                if start.local_name().as_ref() != "xref" {
                    continue;
                }
                let Some(e) = entry.as_mut() else { continue };
                let data = attribute(&start, "data").unwrap_or_default();
                match attribute(&start, "type").as_deref() {
                    Some("rfc") | Some("draft") if !data.is_empty() => {
                        e.documents.push(data.to_ascii_lowercase());
                    }
                    Some("rfc-errata") if !data.is_empty() => e.errata.push(data),
                    _ => {}
                }
            }
            Event::Text(chunk) => push_text(&chunk, &mut open, &mut entry),
            Event::CData(chunk) => push_text(&chunk, &mut open, &mut entry),
            Event::GeneralRef(reference) => {
                if let Some(c) = entity(&reference) {
                    push_text(c.encode_utf8(&mut [0; 4]), &mut open, &mut entry);
                }
            }
            Event::End(end) => {
                if field != Field::Other {
                    if depth_in_field > 0 {
                        depth_in_field = depth_in_field.saturating_sub(1);
                    } else {
                        field = Field::Other;
                    }
                    continue;
                }
                match end.local_name().as_ref() {
                    "record" => {
                        if let (Some(e), Some(r)) = (entry.take(), open.last_mut()) {
                            r.entries.push(e);
                        }
                    }
                    "registry" => done.extend(open.pop()),
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    const XML: &str = r#"<?xml version='1.0' encoding='UTF-8'?>
<registry xmlns="http://www.iana.org/assignments" id="smi-numbers">
  <title>Structure of Management Information (SMI) Numbers</title>
  <registry id="security-smime-0">
    <title>SMI Security for S/MIME Module Identifier (1.2.840.113549.1.9.16.0)</title>
    <xref type="rfc" data="rfc7107"/>
    <record>
      <value>49</value>
      <description>id-mod-MD5-XOR-EXPERIMENT</description>
      <xref type="rfc-errata" data="3866"/>
    </record>
    <record>
      <value>126-200</value>
      <description>Unassigned</description>
    </record>
  </registry>
  <registry id="smi-numbers-1.3.6.1.5.5.7.0">
    <title>SMI Security for PKIX Module Identifier</title>
    <description>iso.org.dod.internet.security.mechanisms.pkix.mod (1.3.6.1.5.5.7.0)</description>
    <record>
      <value>0</value>
      <name>Reserved</name>
    </record>
    <record>
      <value>49</value>
      <description>id-mod-v1AttrCert-02</description>
      <xref type="rfc" data="rfc5912"/>
    </record>
  </registry>
  <registry id="ltans-module-identifiers">
    <title>SMI Security for LTANS Module Identifier</title>
    <description>iso.org.dod.internet.security.mechanisms.ltans.module-identifiers (1.3.6.1.5.5.11.0)</description>
    <record>
      <value>1.3.6.1.5.5.11.0.1</value>
      <description>id-mod-ers</description>
      <xref type="rfc" data="rfc4998"/>
    </record>
  </registry>
  <registry id="security-smime-3">
    <title>SMI Security for S/MIME Algorithms (1.2.840.113549.1.9.16.3)</title>
    <record>
      <value>13</value>
      <description>id-alg-MD5-XOR-EXPERIMENT</description>
      <xref type="rfc" data="rfc6210"/>
    </record>
  </registry>
</registry>"#;

    #[test]
    fn module_identifier_registries_are_read_and_others_skipped() {
        let dir = std::env::temp_dir().join(format!("glint-iana-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("smi-numbers.xml");
        std::fs::write(&path, XML).unwrap();
        let errata = HashMap::from([("3866".to_owned(), "rfc6210".to_owned())]);
        let entries = read_iana(&path, &errata).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        let summary: Vec<(String, &str, Vec<String>)> = entries
            .iter()
            .map(|e| {
                (
                    e.oid.to_string(),
                    e.description.as_str(),
                    e.documents.clone(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                (
                    "1.2.840.113549.1.9.16.0.49".to_owned(),
                    "id-mod-MD5-XOR-EXPERIMENT",
                    vec!["rfc6210".to_owned()]
                ),
                (
                    "1.3.6.1.5.5.7.0.49".to_owned(),
                    "id-mod-v1AttrCert-02",
                    vec!["rfc5912".to_owned()]
                ),
                (
                    "1.3.6.1.5.5.11.0.1".to_owned(),
                    "id-mod-ers",
                    vec!["rfc4998".to_owned()]
                ),
            ]
        );
        assert_eq!(entries[0].errata, ["3866"]);
        assert_eq!(
            entries[1].registry,
            "SMI Security for PKIX Module Identifier"
        );
    }
}
