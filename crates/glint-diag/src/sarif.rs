//! SARIF 2.1.0 output, for code-scanning annotations.

use std::collections::BTreeMap;

use serde_sarif::sarif::{
    ArtifactLocation, Location, Message, MultiformatMessageString, PhysicalLocation, PropertyBag,
    Region, ReportingConfiguration, ReportingDescriptor, Result as SarifResult, ResultLevel, Run,
    Sarif, Tool, ToolComponent,
};

use crate::{Diagnostic, FileId, Files, Label, Severity, Versions, code_info, explain};

const SCHEMA: &str = "https://json.schemastore.org/sarif-2.1.0.json";
const INFORMATION_URI: &str = "https://github.com/glint-asn1/glint";
const RULES_URI: &str = "https://github.com/glint-asn1/glint/blob/main/docs/diagnostics.md";

/// The diagnostics of one checked document and the files they point into. By convention the document itself
/// is the first file, `FileId(0)`; the others are corpus modules.
pub struct SarifInput<'a> {
    pub diagnostics: &'a [Diagnostic],
    pub files: &'a Files,
}

fn level(severity: Severity) -> ResultLevel {
    match severity {
        Severity::Error => ResultLevel::Error,
        Severity::Warning => ResultLevel::Warning,
        Severity::Info => ResultLevel::Note,
    }
}

fn percent_encode(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// A repository-relative URI for the document, or a link to the RFC for a corpus module.
fn artifact_uri(name: &str, is_document: bool) -> String {
    if !is_document && let Some(rest) = name.strip_prefix("rfc") {
        let number: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if !number.is_empty() {
            return format!("https://www.rfc-editor.org/rfc/rfc{number}");
        }
    }
    let unix = name.replace('\\', "/");
    let trimmed = unix.trim_start_matches("./");
    percent_encode(trimmed)
}

fn region(files: &Files, label: &Label) -> Region {
    let (start_line, start_column) = label.line_col;
    let end = files
        .line_index(label.file)
        .map_or((start_line, start_column), |index| {
            index.line_col(label.span.end.max(label.span.start))
        });
    Region::builder()
        .start_line(i64::from(start_line))
        .start_column(i64::from(start_column))
        .end_line(i64::from(end.0))
        .end_column(i64::from(end.1))
        .build()
}

fn location(files: &Files, label: &Label, message: Option<String>) -> Location {
    let document = label.file == FileId(0);
    let physical = PhysicalLocation::builder()
        .artifact_location(
            ArtifactLocation::builder()
                .uri(artifact_uri(files.name(label.file), document))
                .build(),
        )
        .region(region(files, label))
        .build();
    match message {
        Some(text) => Location::builder()
            .physical_location(physical)
            .message(Message::builder().text(text).build())
            .build(),
        None => Location::builder().physical_location(physical).build(),
    }
}

fn describe(label: &Label, files: &Files) -> String {
    let place = format!(
        "{}:{}:{}",
        files.name(label.file),
        label.line_col.0,
        label.line_col.1
    );
    match &label.text {
        Some(text) => format!("{text} ({place})"),
        None => place,
    }
}

fn result(diagnostic: &Diagnostic, files: &Files, rule_index: usize) -> SarifResult {
    let mut text = diagnostic.message.clone();
    for note in &diagnostic.notes {
        text.push('\n');
        text.push_str(note);
    }
    if let Some(help) = &diagnostic.help {
        text.push_str("\nhelp: ");
        text.push_str(help);
    }
    let in_document = diagnostic.primary.file == FileId(0);
    let primary = if in_document {
        location(files, &diagnostic.primary, None)
    } else {
        text.push_str("\nin ");
        text.push_str(&describe(&diagnostic.primary, files));
        let mut anchor = diagnostic.primary.clone();
        anchor.file = FileId(0);
        anchor.span = 0..0;
        anchor.line_col = (1, 1);
        location(files, &anchor, None)
    };
    let related: Vec<Location> = diagnostic
        .secondary
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let mut related = location(files, label, label.text.clone());
            related.id = i64::try_from(i).ok();
            related
        })
        .collect();
    let mut built = SarifResult::builder()
        .message(Message::builder().text(text).build())
        .build();
    built.rule_id = Some(diagnostic.code.to_owned());
    built.rule_index = i64::try_from(rule_index).ok();
    built.level = Some(level(diagnostic.severity));
    built.locations = Some(vec![primary]);
    if !related.is_empty() {
        built.related_locations = Some(related);
    }
    built
}

fn rule(code: &str, severity: Severity) -> ReportingDescriptor {
    let mut built = ReportingDescriptor::builder().id(code.to_owned()).build();
    built.name = Some(code.to_owned());
    built.help_uri = Some(RULES_URI.to_owned());
    let mut configuration = ReportingConfiguration::builder().enabled(true).build();
    configuration.level = Some(serde_json::Value::String(
        match severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "note",
        }
        .to_owned(),
    ));
    built.default_configuration = Some(configuration);
    if let Some(info) = code_info(code) {
        built.short_description = Some(
            MultiformatMessageString::builder()
                .text(info.title.to_owned())
                .build(),
        );
        built.full_description = Some(
            MultiformatMessageString::builder()
                .text(format!("{} (basis: {})", info.title, info.basis))
                .build(),
        );
    }
    if let Some(text) = explain(code) {
        let mut help = MultiformatMessageString::builder()
            .text(text.to_owned())
            .build();
        help.markdown = Some(text.to_owned());
        built.help = Some(help);
    }
    built
}

/// Renders the diagnostics of one or more documents as one SARIF 2.1.0 run.
/// The data build goes in the run's properties as `glintDataBuild`.
pub fn render_sarif(inputs: &[SarifInput<'_>], versions: Versions<'_>) -> String {
    let tool_version = versions.glint;
    let mut rule_codes: BTreeMap<&'static str, (usize, Severity)> = BTreeMap::new();
    for diagnostic in inputs.iter().flat_map(|input| input.diagnostics) {
        rule_codes
            .entry(diagnostic.code)
            .or_insert((0, diagnostic.severity));
    }
    for (index, entry) in rule_codes.values_mut().enumerate() {
        entry.0 = index;
    }
    let rules: Vec<ReportingDescriptor> = rule_codes
        .iter()
        .map(|(code, (_, severity))| {
            rule(
                code,
                code_info(code).map_or(*severity, |info| info.severity),
            )
        })
        .collect();
    let mut results = Vec::new();
    for input in inputs {
        for diagnostic in input.diagnostics {
            let index = rule_codes.get(diagnostic.code).map_or(0, |(i, _)| *i);
            results.push(result(diagnostic, input.files, index));
        }
    }
    let mut driver = ToolComponent::builder().name("glint".to_owned()).build();
    driver.version = Some(tool_version.to_owned());
    driver.semantic_version = Some(tool_version.to_owned());
    driver.information_uri = Some(INFORMATION_URI.to_owned());
    driver.rules = Some(rules);
    let mut run = Run::builder()
        .tool(Tool::builder().driver(driver).build())
        .build();
    run.column_kind = Some(serde_json::Value::String("unicodeCodePoints".to_owned()));
    run.results = Some(results);
    if let Some(data) = versions.data {
        let mut properties = PropertyBag::builder().build();
        properties.additional_properties.insert(
            "glintDataBuild".to_owned(),
            serde_json::Value::String(data.to_owned()),
        );
        run.properties = Some(properties);
    }
    let mut sarif = Sarif::builder()
        .version(serde_json::Value::String("2.1.0".to_owned()))
        .runs(vec![run])
        .build();
    sarif.schema = Some(SCHEMA.to_owned());
    serde_json::to_string_pretty(&sarif).unwrap_or_else(|_| "{}".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(path: &str) -> serde_json::Value {
        let mut files = Files::new();
        let (doc, index) = files.add_document(path, "line one\nA ::= B\n");
        let (module, module_index) = files.add("rfc5280#PKIX1Explicit88", "X ::= INTEGER\n");
        let diagnostic = Diagnostic::new("E300", "differs", Label::new(doc, 9..10, &index))
            .with_secondary(Label::new(module, 0..1, &module_index).with_text("defined here"))
            .with_help("fix it");
        let info = Diagnostic::new("I141", "placeholder", Label::new(doc, 0..4, &index));
        let diagnostics = [diagnostic, info];
        let input = SarifInput {
            diagnostics: &diagnostics,
            files: &files,
        };
        serde_json::from_str(&render_sarif(
            &[input],
            Versions {
                glint: "1.2.3",
                data: Some("2026-11-05.0123abcd"),
            },
        ))
        .unwrap()
    }

    #[test]
    fn the_run_names_the_tool_and_its_rules() {
        let sarif = render("./dir with space/d.xml");
        assert_eq!(sarif["version"], "2.1.0");
        let driver = &sarif["runs"][0]["tool"]["driver"];
        assert_eq!(driver["name"], "glint");
        assert_eq!(driver["version"], "1.2.3");
        assert_eq!(
            sarif["runs"][0]["properties"]["glintDataBuild"],
            "2026-11-05.0123abcd"
        );
        let ids: Vec<_> = driver["rules"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["E300", "I141"]);
        assert_eq!(driver["rules"][0]["defaultConfiguration"]["level"], "error");
        assert_eq!(sarif["runs"][0]["columnKind"], "unicodeCodePoints");
    }

    #[test]
    fn results_carry_level_region_and_related_location() {
        let sarif = render("./dir with space/d.xml");
        let first = &sarif["runs"][0]["results"][0];
        assert_eq!(first["ruleId"], "E300");
        assert_eq!(first["ruleIndex"], 0);
        assert_eq!(first["level"], "error");
        let physical = &first["locations"][0]["physicalLocation"];
        assert_eq!(
            physical["artifactLocation"]["uri"],
            "dir%20with%20space/d.xml"
        );
        assert_eq!(physical["region"]["startLine"], 2);
        assert_eq!(physical["region"]["startColumn"], 1);
        assert_eq!(physical["region"]["endColumn"], 2);
        assert!(
            first["message"]["text"]
                .as_str()
                .unwrap()
                .contains("help: fix it")
        );
        let related = &first["relatedLocations"][0];
        assert_eq!(
            related["physicalLocation"]["artifactLocation"]["uri"],
            "https://www.rfc-editor.org/rfc/rfc5280"
        );
        assert_eq!(related["message"]["text"], "defined here");
        assert_eq!(sarif["runs"][0]["results"][1]["level"], "note");
    }

    #[test]
    fn no_diagnostics_still_make_a_valid_run() {
        let files = Files::new();
        let input = SarifInput {
            diagnostics: &[],
            files: &files,
        };
        let sarif: serde_json::Value = serde_json::from_str(&render_sarif(
            &[input],
            Versions {
                glint: "0",
                data: None,
            },
        ))
        .unwrap();
        assert_eq!(sarif["runs"][0]["results"], serde_json::json!([]));
    }
}
