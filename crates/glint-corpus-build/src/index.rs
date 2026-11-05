//! Reads `rfc-index.xml` for each RFC's status, publication month and the RFCs that obsolete it.

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
