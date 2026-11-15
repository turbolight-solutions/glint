//! Glint's schema-driven decoder (Codegen and Decoder Design, Decoder).
//!
//! The decoder walks a BER or DER encoding the way `openssl asn1parse` does and, given a root type of a
//! [`glint_ir::IrSet`], names every TLV after the component and type it encodes, renders its value, decodes the contents
//! of `CONTAINING` octet strings and of open types through their tables, and reports what breaks BER, DER, the tags or
//! the constraints. A TLV that does not match its type is decoded without a schema, and decoding carries on with its
//! siblings. The crate does no I/O: corpus lookups go through an [`Oracle`].

mod ber;
mod finding;
mod input;
mod text;
mod value;
mod walk;

use std::rc::Rc;

use glint_diag::Severity;
use glint_ir::{IrSet, Type, TypeId};
use serde::Serialize;

pub use ber::{Length, Tag};
pub use finding::DecodeFinding;
pub use input::{
    Encoding, Input, base64, base64_encode, canonical_label, common_roots, label_roots, pem_armor,
    pem_blocks, read_as, sniff,
};
pub use text::render_text;
pub use value::{OidName, Rendered};

/// How a TLV was decoded.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "lowercase", tag = "state")]
pub enum SchemaState {
    /// As the type says.
    #[default]
    Matched,
    /// Without a schema: the TLV does not match its type, its type is not known (an open type no table resolves), or
    /// no type was given.
    Unmatched,
    /// As the type the OID hint table names for a sibling OID: a heuristic for modules that have no tables.
    Hinted {
        /// The module whose object set names the type (`rfc5912#PKIX1Implicit-2009`).
        from: String,
    },
}

/// One TLV.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Tlv {
    pub offset: usize,
    pub header_len: usize,
    pub len: Length,
    /// The end of the TLV, end-of-contents octets included.
    pub end: usize,
    /// The identifier, absent for octets that do not start a TLV.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<Tag>,
    /// How the type tags this TLV, when it is a tag written in (or given automatically by) the module.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tagging: Option<glint_ir::TagMode>,
    /// The component, or `component.alternative` for a `CHOICE`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    /// `Module.Type` as declared, with the actual parameters of an instantiation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Rendered>,
    /// The TLVs of a constructed encoding, or of the contents of a `CONTAINING` octet string.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Tlv>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<DecodeFinding>,
    pub schema: SchemaState,
}

impl Tlv {
    /// This TLV and every TLV below it, depth first.
    pub fn walk(&self) -> Vec<&Tlv> {
        let mut out = vec![self];
        for child in &self.children {
            out.extend(child.walk());
        }
        out
    }
}

/// What a decode found.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Decoded {
    /// The top-level TLV, then any that follow it.
    pub tlvs: Vec<Tlv>,
    /// Findings on the input as a whole (trailing data).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<DecodeFinding>,
}

impl Decoded {
    /// Every finding, those of the TLVs first.
    pub fn all_findings(&self) -> Vec<&DecodeFinding> {
        let mut out: Vec<&DecodeFinding> = self
            .tlvs
            .iter()
            .flat_map(|t| t.walk())
            .flat_map(|t| &t.findings)
            .collect();
        out.extend(&self.findings);
        out
    }

    /// Whether the encoding has errors; with `strict_der`, DER violations count too.
    pub fn failed(&self, strict_der: bool) -> bool {
        self.all_findings()
            .iter()
            .any(|f| f.severity == Severity::Error || (strict_der && f.code == "W402"))
    }
}

/// A type the OID hint table names for an OID, ready to decode with.
#[derive(Debug, Clone)]
pub struct Hint {
    /// `rfc5912#PKIX1Implicit-2009`.
    pub from: String,
    /// The class field (`EXTENSION.&ExtnType`), for messages.
    pub field: String,
    pub set: Rc<IrSet>,
    pub ty: Type,
}

/// What the decoder asks of the corpus.
pub trait Oracle {
    /// The names the corpus gives the OID, preferred first.
    fn oid_names(&self, _dotted: &str) -> Vec<OidName> {
        Vec::new()
    }

    /// The types the hint table names for objects keyed by the OID, preferred first.
    fn hints(&self, _dotted: &str) -> Vec<Hint> {
        Vec::new()
    }
}

/// An oracle that knows nothing.
pub struct NoOracle;

impl Oracle for NoOracle {}

/// Decodes `bytes` as the type `root` of `set`, or without a schema when `schema` is `None`.
pub fn decode(bytes: &[u8], schema: Option<(&IrSet, TypeId)>, oracle: &dyn Oracle) -> Decoded {
    walk::decode(bytes, schema, oracle)
}
