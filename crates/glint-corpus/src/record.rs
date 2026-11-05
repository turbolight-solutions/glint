use serde::{Deserialize, Serialize};

use crate::{AllowedOid, Oid, OidNode};

/// Which grammar family a module belongs to (design spec, "Module record").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SyntaxEra {
    /// Uses `ANY`.
    #[serde(rename = "1988")]
    Y1988,
    /// Defines or uses `MACRO` notation.
    #[serde(rename = "1988-macro")]
    Y1988Macro,
    /// Uses X.681 constructs.
    #[serde(rename = "2002+")]
    Y2002,
    Neutral,
}

/// The result of parsing and checking a corpus module with the full corpus available.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParseStatus {
    Clean,
    Warnings,
    Errors,
    Unparseable,
}

/// How a module's text was obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OverrideTier {
    Extraction,
    ErrataVerified,
    Correction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SourceKind {
    /// Extracted from the prose of the document (an RFC).
    Extracted,
    /// Taken whole from a standalone module file published by the standards body.
    Standalone,
    Overridden {
        tier: OverrideTier,
        errata_ids: Vec<String>,
    },
}

/// The standards body of RFCs. Other bodies are named by the sources manifest (`itu-t`, `etsi`, ...).
pub const BODY_RFC: &str = "rfc";
/// Internet-Drafts: active drafts at their latest revision.
pub const BODY_DRAFT: &str = "draft";

/// Where a module was published: the document of a standards body, whatever the way its text was obtained.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleSource {
    /// The standards body: `rfc`, `itu-t`, `etsi`.
    pub body: String,
    /// The document within the body: `rfc5280`, `X.509`, `TS 102 941`.
    pub document: String,
    /// The edition of the document, where the body has them (`10/2012`).
    pub edition: Option<String>,
    /// Where the module's text came from, when it was downloaded.
    pub url: Option<String>,
    pub section: Option<String>,
    pub kind: SourceKind,
}

impl ModuleSource {
    /// A module of RFC `number`.
    pub fn rfc(number: u32, section: Option<String>, kind: SourceKind) -> Self {
        Self {
            body: BODY_RFC.to_owned(),
            document: format!("rfc{number}"),
            edition: None,
            url: None,
            section,
            kind,
        }
    }

    /// A module of the Internet-Draft `document` (`draft-ietf-lamps-foo-03`), revised on `date` (`YYYY-MM-DD`).
    pub fn draft(document: &str, date: &str, section: Option<String>, kind: SourceKind) -> Self {
        Self {
            body: BODY_DRAFT.to_owned(),
            document: document.to_owned(),
            edition: Some(date.to_owned()),
            url: None,
            section,
            kind,
        }
    }

    pub fn is_draft(&self) -> bool {
        self.body == BODY_DRAFT
    }

    /// `rfc5280`, or `X.509`: the document id, which is also the prefix of diagnostics file names.
    pub fn label(&self) -> String {
        self.document.clone()
    }

    /// `RFC 5280`, or `X.509 (10/2012)`.
    pub fn title(&self) -> String {
        match &self.edition {
            Some(edition) => format!("{} ({edition})", document_title(&self.document)),
            None => document_title(&self.document),
        }
    }

    /// The RFC number, for a module of an RFC.
    pub fn rfc_number(&self) -> Option<u32> {
        if self.body == BODY_RFC {
            rfc_number(&self.document)
        } else {
            None
        }
    }

    /// How recent the document is within its body: the number of an RFC, the revision date of a draft as
    /// `YYYYMMDD`. Import resolution prefers the larger value, and falls back to the highest OID.
    pub fn recency(&self) -> Option<u32> {
        if self.is_draft() {
            let digits: String = self
                .edition
                .as_deref()?
                .chars()
                .filter(char::is_ascii_digit)
                .collect();
            digits.parse().ok()
        } else {
            self.rfc_number()
        }
    }
}

/// The number in an RFC document id (`rfc5280`), compared without regard to case.
pub fn rfc_number(document: &str) -> Option<u32> {
    let (prefix, digits) = document.split_at_checked(3)?;
    if prefix.eq_ignore_ascii_case("rfc") && digits.bytes().all(|b| b.is_ascii_digit()) {
        digits.parse().ok()
    } else {
        None
    }
}

/// Whether two document ids name the same document: equal ignoring case, or two revisions of one draft.
pub fn same_document(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b) || draft_name(a).is_some_and(|name| draft_name(b) == Some(name))
}

/// `draft-ietf-lamps-foo` for `draft-ietf-lamps-foo-03`.
pub fn draft_name(document: &str) -> Option<&str> {
    let (name, rev) = document.rsplit_once('-')?;
    (name.starts_with("draft-") && rev.len() == 2 && rev.bytes().all(|b| b.is_ascii_digit()))
        .then_some(name)
}

/// `RFC 5280` for the document id `rfc5280`; other ids are titles already.
pub fn document_title(document: &str) -> String {
    match rfc_number(document) {
        Some(number) => format!("RFC {number}"),
        None => document.to_owned(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Exports {
    /// No `EXPORTS` clause, or `EXPORTS ALL`.
    All,
    List(Vec<String>),
}

/// One module of the corpus.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleRecord {
    pub name: String,
    pub oid: Option<Oid>,
    pub source: ModuleSource,
    pub rfc_status: Option<String>,
    pub obsoleted_by: Vec<u32>,
    pub syntax_era: SyntaxEra,
    pub text: String,
    pub exports: Exports,
    pub parse_status: ParseStatus,
    pub sha256: String,
}

impl ModuleRecord {
    /// `rfc5280#PKIX1Explicit88` or `X.509#InformationFramework`, the file name used in diagnostics.
    pub fn label(&self) -> String {
        format!("{}#{}", self.source.label(), self.name)
    }

    /// How import resolution by name ranks the module, higher first: a current RFC or a module of another standards
    /// body, then an active draft, then an obsoleted RFC; within each, the more recent document.
    pub fn rank(&self) -> (u8, Option<u32>) {
        let tier = if self.source.is_draft() {
            1
        } else if self.obsoleted_by.is_empty() {
            2
        } else {
            0
        };
        (tier, self.source.recency())
    }

    /// Whether import resolution by name may use the module: not a draft module with syntax errors.
    pub fn resolvable_by_name(&self) -> bool {
        !(self.source.is_draft() && self.parse_status == ParseStatus::Unparseable)
    }
}

/// A value assignment of type `OBJECT IDENTIFIER`, evaluated to dotted form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OidAssignment {
    pub oid: Oid,
    pub name: String,
    pub module: String,
    /// The document id of the module that makes the assignment.
    pub document: String,
}

/// An entry of an IANA registry of ASN.1 module identifiers (the "SMI Security for ... Module Identifier"
/// registries): the authority for which module an identifier in those arcs belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IanaModuleId {
    pub oid: Oid,
    /// What the registry lists: usually the label of the last arc (`id-mod-v1AttrCert-02`), sometimes the module name.
    pub description: String,
    /// The registry's title.
    pub registry: String,
    /// The documents the entry cites, as document ids (`rfc5912`, `draft-ietf-lamps-foo`), errata resolved to the
    /// document they are reported against.
    pub documents: Vec<String>,
    /// The errata the entry cites (IANA assigns some identifiers through an erratum).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errata: Vec<String>,
}

impl IanaModuleId {
    /// `RFC 6210 (erratum 3866)`, or `RFC 5912 and RFC 6268`: whom IANA registers the identifier for.
    pub fn registrant(&self) -> String {
        let documents: Vec<String> = self.documents.iter().map(|d| document_title(d)).collect();
        let mut text = documents.join(" and ");
        if !self.errata.is_empty() {
            let word = if self.errata.len() > 1 {
                "errata"
            } else {
                "erratum"
            };
            text = format!("{text} ({word} {})", self.errata.join(", "));
        }
        text
    }
}

/// Identifies a build of the data.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CorpusInfo {
    /// `<UTC date>.<hash of the content>`, set once by `corpus-build`; everything downstream reads it from here.
    pub build: String,
    pub notices: Vec<String>,
}

/// A document that publishes modules of the corpus.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentRecord {
    /// `rfc5280`, `X.509`, `draft-ietf-lamps-foo-03`.
    pub id: String,
    pub body: String,
    pub status: Option<String>,
    pub obsoleted_by: Vec<u32>,
    /// `YYYY-MM-DD` (drafts) or `YYYY-MM` (RFCs) of publication, where known.
    pub date: Option<String>,
    /// `xml` or `text` when `documents.bin` has the text of the document.
    pub text_format: Option<String>,
}

/// `corpus.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorpusData {
    pub schema: u32,
    pub info: CorpusInfo,
    pub documents: Vec<DocumentRecord>,
    pub modules: Vec<ModuleRecord>,
    pub oid_assignments: Vec<OidAssignment>,
    /// The label table (value names are in `oid_assignments`, module identifiers in `modules`).
    pub oid_nodes: Vec<OidNode>,
    /// Wrong but deployed OIDs, whose subtrees the label checks leave alone.
    pub oid_allowlist: Vec<AllowedOid>,
    /// The IANA registries of module identifiers; absent from data built without them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub iana_module_ids: Vec<IanaModuleId>,
}

impl Default for CorpusData {
    fn default() -> Self {
        Self {
            schema: crate::SCHEMA,
            info: CorpusInfo::default(),
            documents: Vec::new(),
            modules: Vec::new(),
            oid_assignments: Vec::new(),
            oid_nodes: Vec::new(),
            oid_allowlist: Vec::new(),
            iana_module_ids: Vec::new(),
        }
    }
}
