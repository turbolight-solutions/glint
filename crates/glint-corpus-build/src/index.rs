//! Reads `rfc-index.xml` for each RFC's status, publication month and the RFCs that obsolete it, and `all_id2.txt` for
//! the active Internet-Drafts.

use std::collections::HashMap;
use std::path::Path;

use quick_xml::Reader;
use quick_xml::events::Event;

#[derive(Debug, Clone, Default)]
pub struct RfcMeta {
    pub status: Option<String>,
    pub obsoleted_by: Vec<u32>,
    /// `YYYY-MM`.
    pub date: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    DocId,
    Status,
    Month,
    Year,
    Other,
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

fn rfc_number(doc_id: &str) -> Option<u32> {
    doc_id.strip_prefix("RFC")?.trim().parse().ok()
}

pub fn read_index(path: &Path) -> Result<HashMap<u32, RfcMeta>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut reader = Reader::from_str(&text);
    let mut out: HashMap<u32, RfcMeta> = HashMap::new();
    let mut current: Option<(Option<u32>, RfcMeta)> = None;
    let mut in_obsoleted_by = false;
    let mut field = Field::Other;
    let mut in_date = false;
    let mut month: Option<usize> = None;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => match e.name().as_ref() {
                "rfc-entry" => current = Some((None, RfcMeta::default())),
                "obsoleted-by" => in_obsoleted_by = true,
                "doc-id" => field = Field::DocId,
                "current-status" => field = Field::Status,
                "date" => in_date = true,
                "month" if in_date => field = Field::Month,
                "year" if in_date => field = Field::Year,
                _ => field = Field::Other,
            },
            Ok(Event::Text(t)) => {
                let value = t.trim().to_owned();
                if let Some((number, meta)) = current.as_mut() {
                    match field {
                        Field::DocId if in_obsoleted_by => {
                            if let Some(n) = rfc_number(&value) {
                                meta.obsoleted_by.push(n);
                            }
                        }
                        Field::DocId => *number = rfc_number(&value),
                        Field::Status => meta.status = Some(value.to_lowercase()),
                        Field::Month => {
                            month = MONTHS.iter().position(|m| *m == value);
                        }
                        Field::Year => {
                            if let Some(index) = month.take() {
                                meta.date = Some(format!("{value}-{:02}", index.saturating_add(1)));
                            }
                        }
                        Field::Other => {}
                    }
                }
            }
            Ok(Event::End(e)) => match e.name().as_ref() {
                "obsoleted-by" => in_obsoleted_by = false,
                "date" => in_date = false,
                "rfc-entry" => {
                    if let Some((Some(n), meta)) = current.take() {
                        out.insert(n, meta);
                    }
                }
                _ => field = Field::Other,
            },
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("{}: {e}", path.display())),
            _ => {}
        }
    }
    Ok(out)
}

/// An active Internet-Draft at its latest revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftEntry {
    /// `draft-ietf-lamps-foo-03`.
    pub id: String,
    /// The IESG state, such as `I-D Exists` or `In Last Call`.
    pub iesg_state: Option<String>,
    /// `YYYY-MM-DD` of the revision.
    pub date: String,
}

/// The active drafts listed in `all_id2.txt` (tab-separated: name and revision, -1, state, IESG state, RFC number,
/// replacement, revision date, ...).
pub fn read_drafts(path: &Path) -> Result<Vec<DraftEntry>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let text = String::from_utf8_lossy(&bytes);
    let mut drafts: Vec<DraftEntry> =
        text.lines()
            .filter(|line| !line.starts_with('#'))
            .filter_map(|line| {
                let fields: Vec<&str> = line.split('\t').collect();
                let (id, state, iesg, date) = (
                    fields.first()?.trim(),
                    fields.get(2)?.trim(),
                    fields.get(3)?.trim(),
                    fields.get(6)?.trim(),
                );
                (state == "Active" && glint_corpus::draft_name(id).is_some() && date.len() == 10)
                    .then(|| DraftEntry {
                        id: id.to_owned(),
                        iesg_state: (!iesg.is_empty()).then(|| iesg.to_owned()),
                        date: date.to_owned(),
                    })
            })
            .collect();
    drafts.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(drafts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_active_drafts_are_read() {
        let dir = std::env::temp_dir().join(format!("glint-all-id-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("all_id2.txt");
        std::fs::write(
            &path,
            "# header\n\
             draft-ietf-lamps-foo-03\t-1\tActive\tIn Last Call\t\t\t2026-10-20\tlamps\n\
             draft-old-thing-07\t-1\tExpired\t\t\t\t2019-01-01\n\
             draft-ind-bar-00\t-1\tActive\t\t\t\t2026-11-01\n",
        )
        .unwrap();
        let drafts = read_drafts(&path).unwrap();
        assert_eq!(
            drafts,
            vec![
                DraftEntry {
                    id: "draft-ietf-lamps-foo-03".into(),
                    iesg_state: Some("In Last Call".into()),
                    date: "2026-10-20".into(),
                },
                DraftEntry {
                    id: "draft-ind-bar-00".into(),
                    iesg_state: None,
                    date: "2026-11-01".into(),
                },
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
