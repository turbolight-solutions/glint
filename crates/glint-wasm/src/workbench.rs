//! What the workbench asks of Glint (Codegen and Decoder Design, Web: `#/workbench`): decodes of PEM, base64 or DER as
//! a type of a corpus module or of the checker's document, the types a module offers, the example artifacts, and the
//! code the backends generate for a type.

use glint_check::{CorpusOracle, lower_corpus_root, lower_corpus_type, lower_user_type};
use glint_codegen::{BackendMeta, File, Finding, Options, backend, backends, generate};
use glint_corpus::{Artifact, ArtifactsData, BrokenArtifact, Confidence, CorpusProvider};
use std::path::Path;

use glint_decode::{
    DecodeFinding, Encoding, Input, Tlv, base64, base64_encode, base64_lines, canonical_label,
    decode, label_roots, pem_armor, read_as, sniff,
};
use glint_extract::{BlockKind, Format, detect_format, extract};
use glint_ir::IrSet;
use glint_sema::{ModuleModel, SymbolKind};
use glint_syntax::{ParseOptions, ast, parse_source_with};
use serde::{Deserialize, Serialize};

use crate::service::{FormatName, Result, Service, ServiceError};

/// How the input is written; `auto` reads PEM, then base64, then DER.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InputFormat {
    #[default]
    Auto,
    Pem,
    Base64,
    Der,
}

/// A corpus module, as the corpus browser names it.
#[derive(Debug, Clone, Deserialize)]
pub struct ModuleSpec {
    pub source: String,
    pub name: String,
    #[serde(default)]
    pub oid: Option<String>,
}

/// A document whose ASN.1 modules a decode can take its type from: the one open in the checker.
#[derive(Debug, Clone, Deserialize)]
pub struct UserDocument {
    pub content: String,
    #[serde(default)]
    pub name: Option<String>,
    /// As in `check`; detected when absent.
    #[serde(default)]
    pub format: Option<FormatName>,
}

impl UserDocument {
    /// The texts of the document's modules, extracted as `check` extracts them.
    fn module_texts(&self) -> Vec<String> {
        let name = self.name.as_deref().unwrap_or("document");
        let format = self.format.map_or_else(
            || detect_format(Some(Path::new(name)), &self.content),
            Format::from,
        );
        extract(&self.content, format)
            .blocks
            .into_iter()
            .filter(|b| b.kind != BlockKind::Fragment)
            .map(|b| b.text)
            .collect()
    }
}

/// Where the root type comes from: a corpus module, or (with `document`) the module `module.name` of user ASN.1.
#[derive(Debug, Clone, Deserialize)]
pub struct SchemaSpec {
    #[serde(default)]
    pub module: Option<ModuleSpec>,
    /// The checker's document; `module.name` then names one of its modules, and `module.source` is ignored.
    #[serde(default)]
    pub document: Option<UserDocument>,
}

/// `{ input?, module?, document?, type?, no_schema? }`; the octets travel beside it.
#[derive(Debug, Clone, Deserialize)]
pub struct DecodeRequest {
    #[serde(default)]
    pub input: InputFormat,
    #[serde(flatten)]
    pub schema: SchemaSpec,
    /// The root type; without it, each PEM block's label chooses one.
    #[serde(default, rename = "type")]
    pub ty: Option<String>,
    /// Decode without a schema even when a PEM label suggests one.
    #[serde(default)]
    pub no_schema: bool,
}

/// `{ module?, document?, type, backend }`: the code of a root type, for one backend.
#[derive(Debug, Clone, Deserialize)]
pub struct CodegenRequest {
    #[serde(flatten)]
    pub schema: SchemaSpec,
    #[serde(rename = "type")]
    pub ty: String,
    pub backend: String,
}

#[derive(Serialize)]
pub struct CodegenResponse {
    backend: &'static BackendMeta,
    /// `doc#Module.Type` or `Module.Type`.
    root: String,
    files: Vec<File>,
    findings: Vec<Finding>,
}

#[derive(Serialize)]
pub struct BackendsResponse {
    backends: Vec<&'static BackendMeta>,
}

/// One decoded input: its octets (for the hex view) and its TLV tree.
#[derive(Serialize)]
pub struct DecodedBlock {
    encoding: Encoding,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    /// The root type, `doc#Module.Type` or `Module.Type`; absent for a decode without a schema.
    #[serde(skip_serializing_if = "Option::is_none")]
    root: Option<String>,
    /// The octets, base64.
    der: String,
    tlvs: Vec<Tlv>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    findings: Vec<DecodeFinding>,
}

#[derive(Serialize)]
pub struct DecodeResponse {
    blocks: Vec<DecodedBlock>,
}

#[derive(Serialize)]
pub struct ModuleTypes {
    name: String,
    /// The types a decode can start from: the module's unparameterized type assignments, in order.
    types: Vec<String>,
}

#[derive(Serialize)]
pub struct TypesResponse {
    modules: Vec<ModuleTypes>,
}

#[derive(Serialize)]
pub struct ArtifactsInfo {
    build: String,
    artifacts: usize,
    broken: usize,
}

/// `{ document?, label?, type?, confidence?, errors? }`; every given filter must match.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ArtifactQuery {
    #[serde(default)]
    pub document: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    /// Text the identified type contains (`Certificate`, `rfc5912#`).
    #[serde(default, rename = "type")]
    pub ty: Option<String>,
    #[serde(default)]
    pub confidence: Option<Confidence>,
    /// Only the examples identified as a type whose decode has errors (erratum candidates); an `unknown` example's
    /// errors are its closest candidate's.
    #[serde(default)]
    pub errors: bool,
}

/// An artifact without its octets, and their size.
#[derive(Serialize)]
pub struct ArtifactRow<'a> {
    #[serde(flatten)]
    artifact: ArtifactSummary<'a>,
    size: usize,
}

#[derive(Serialize)]
struct ArtifactSummary<'a> {
    id: &'a str,
    document: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    section: Option<&'a str>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    appendix: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    caption: Option<&'a str>,
    line: u32,
    label: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    root: Option<&'a str>,
    confidence: Confidence,
    #[serde(skip_serializing_if = "<[String]>::is_empty")]
    findings: &'a [String],
    #[serde(skip_serializing_if = "Option::is_none")]
    first_error: Option<&'a str>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    obsoleted: bool,
}

#[derive(Serialize)]
pub struct ArtifactList<'a> {
    build: &'a str,
    artifacts: Vec<ArtifactRow<'a>>,
    /// The broken blocks of the documents the query names (all of them without a `document` filter); other filters
    /// do not apply to them.
    broken: Vec<&'a BrokenArtifact>,
    /// Every label of the data, for the filter.
    labels: Vec<&'a str>,
}

/// One artifact in the forms it downloads as.
#[derive(Serialize)]
pub struct ArtifactDetail<'a> {
    #[serde(flatten)]
    row: ArtifactRow<'a>,
    /// The octets, base64 on one line.
    der: String,
    /// The canonical label it is served with, when the PEM label map knows its label.
    #[serde(skip_serializing_if = "Option::is_none")]
    canonical_label: Option<&'static str>,
    /// What a download gives: PEM with the canonical label, or base64 in 64-column lines for an unknown label.
    text: String,
}

fn summary(artifact: &Artifact) -> ArtifactRow<'_> {
    ArtifactRow {
        artifact: ArtifactSummary {
            id: &artifact.id,
            document: &artifact.document,
            section: artifact.section.as_deref(),
            appendix: artifact.appendix,
            title: artifact.title.as_deref(),
            caption: artifact.caption.as_deref(),
            line: artifact.line,
            label: &artifact.label,
            root: artifact.root.as_deref(),
            confidence: artifact.confidence,
            findings: &artifact.findings,
            first_error: artifact.first_error.as_deref(),
            obsoleted: artifact.obsoleted,
        },
        // Base64 holds three octets in four characters, less the padding.
        size: (artifact.der.len() / 4).saturating_mul(3).saturating_sub(
            artifact
                .der
                .bytes()
                .rev()
                .take_while(|&b| b == b'=')
                .count(),
        ),
    }
}

/// A root type and how to name it.
struct Schema {
    set: IrSet,
    name: String,
}

/// The type names of the module definitions in `text`, all of them or only the one named `only`.
fn module_types(text: &str, options: ParseOptions, only: Option<&str>) -> Vec<ModuleTypes> {
    let parse = parse_source_with(text, options);
    let Some(file) = ast::SourceFile::cast(parse.syntax()) else {
        return Vec::new();
    };
    file.modules()
        .filter_map(|module| ModuleModel::build(&module))
        .filter(|model| only.is_none_or(|name| model.name == name))
        .map(|model| ModuleTypes {
            types: model
                .symbols
                .iter()
                .filter(|s| s.kind == SymbolKind::Type && !s.is_parameterized())
                .map(|s| s.name.clone())
                .collect(),
            name: model.name,
        })
        .collect()
}

impl Service {
    fn artifacts_data(&self) -> Result<&ArtifactsData> {
        self.artifacts.as_ref().ok_or_else(|| {
            ServiceError::new(
                "artifacts_loading",
                "the example artifacts are still loading; try again in a moment",
            )
        })
    }

    /// Takes `artifacts.json`, which must be from the build of the corpus.
    pub fn load_artifacts(&mut self, json: &[u8]) -> Result<ArtifactsInfo> {
        let data = ArtifactsData::from_json(json)?;
        data.require_build(&self.corpus().info().build)?;
        let info = ArtifactsInfo {
            build: data.build.clone(),
            artifacts: data.artifacts.len(),
            broken: data.broken.len(),
        };
        self.artifacts = Some(data);
        Ok(info)
    }

    pub fn has_artifacts(&self) -> bool {
        self.artifacts.is_some()
    }

    /// The artifacts the query selects, in the data's order (by document, then position).
    pub fn artifacts(&self, query: &ArtifactQuery) -> Result<ArtifactList<'_>> {
        let data = self.artifacts_data()?;
        let document = query.document.as_deref().filter(|d| !d.is_empty());
        let label = query.label.as_deref().filter(|l| !l.is_empty());
        let ty = query
            .ty
            .as_deref()
            .filter(|t| !t.is_empty())
            .map(str::to_lowercase);
        let artifacts = data
            .artifacts
            .iter()
            .filter(|a| document.is_none_or(|d| a.document.eq_ignore_ascii_case(d)))
            .filter(|a| label.is_none_or(|l| a.label.eq_ignore_ascii_case(l)))
            .filter(|a| {
                ty.as_ref().is_none_or(|t| {
                    a.root
                        .as_ref()
                        .is_some_and(|r| r.to_lowercase().contains(t))
                })
            })
            .filter(|a| query.confidence.is_none_or(|c| a.confidence == c))
            .filter(|a| !query.errors || (a.has_errors() && a.confidence != Confidence::Unknown))
            .map(summary)
            .collect();
        let broken = data
            .broken
            .iter()
            .filter(|b| document.is_none_or(|d| b.document.eq_ignore_ascii_case(d)))
            .collect();
        let mut labels: Vec<&str> = data.artifacts.iter().map(|a| a.label.as_str()).collect();
        labels.sort_unstable();
        labels.dedup();
        Ok(ArtifactList {
            build: &data.build,
            artifacts,
            broken,
            labels,
        })
    }

    /// One artifact with its octets: PEM when its label is a known one, else base64 of the octets.
    pub fn artifact(&self, id: &str) -> Result<ArtifactDetail<'_>> {
        let data = self.artifacts_data()?;
        let artifact = data
            .get(id)
            .ok_or_else(|| ServiceError::not_found(format!("no example artifact `{id}`")))?;
        let der = base64(artifact.der.as_bytes()).ok_or_else(|| {
            ServiceError::new("data_damaged", format!("the octets of `{id}` are damaged"))
        })?;
        let canonical = canonical_label(&artifact.label);
        let text = match canonical {
            Some(label) => pem_armor(label, &der),
            None => base64_lines(&der),
        };
        Ok(ArtifactDetail {
            row: summary(artifact),
            der: base64_encode(&der),
            canonical_label: canonical,
            text,
        })
    }

    /// The types a decode can start from: those of one corpus module, or of every module of the user ASN.1.
    pub fn types(&self, spec: &SchemaSpec) -> Result<TypesResponse> {
        if let Some(document) = &spec.document {
            let only = spec.module.as_ref().map(|m| m.name.as_str());
            let modules = document
                .module_texts()
                .iter()
                .flat_map(|text| module_types(text, ParseOptions::default(), only))
                .collect();
            return Ok(TypesResponse { modules });
        }
        let module = spec
            .module
            .as_ref()
            .ok_or_else(|| ServiceError::bad_request("give a `module` or `asn1`"))?;
        let index = self.module_index(&module.source, &module.name, module.oid.as_deref())?;
        let record = self
            .corpus()
            .modules()
            .get(index)
            .ok_or_else(|| ServiceError::not_found("no such module"))?;
        Ok(TypesResponse {
            modules: module_types(&record.text, ParseOptions::corpus(), Some(&record.name)),
        })
    }

    fn schema(&self, spec: &SchemaSpec, ty: &str) -> Result<Schema> {
        let module = spec
            .module
            .as_ref()
            .ok_or_else(|| ServiceError::bad_request("a `type` needs a `module`"))?;
        if let Some(document) = &spec.document {
            let set = lower_user_type(&document.module_texts(), self.corpus(), &module.name, ty)
                .map_err(ServiceError::not_found)?;
            return Ok(Schema {
                set,
                name: format!("{}.{ty}", module.name),
            });
        }
        let index = self.module_index(&module.source, &module.name, module.oid.as_deref())?;
        let set = lower_corpus_type(self.corpus(), index, ty).map_err(ServiceError::not_found)?;
        let record = self
            .corpus()
            .modules()
            .get(index)
            .ok_or_else(|| ServiceError::not_found("no such module"))?;
        Ok(Schema {
            set,
            name: format!("{}#{}.{ty}", record.source.document, record.name),
        })
    }

    /// The first corpus root of a PEM label that the corpus can lower.
    fn schema_for_label(&self, label: &str) -> Option<Schema> {
        label_roots(label).iter().find_map(|root| {
            lower_corpus_root(self.corpus(), root)
                .ok()
                .map(|set| Schema {
                    set,
                    name: (*root).to_owned(),
                })
        })
    }

    /// Decodes `bytes` (PEM or base64 text, or DER) as the request's type, or each PEM block as the type its label
    /// suggests.
    pub fn decode(&self, request: &DecodeRequest, bytes: &[u8]) -> Result<DecodeResponse> {
        let inputs: Vec<Input> = match request.input {
            InputFormat::Auto => Ok(sniff(bytes)),
            InputFormat::Pem => read_as(bytes, Encoding::Pem),
            InputFormat::Base64 => read_as(bytes, Encoding::Base64),
            InputFormat::Der => read_as(bytes, Encoding::Der),
        }
        .map_err(ServiceError::bad_request)?;
        let fixed = match request.ty.as_deref().filter(|t| !t.is_empty()) {
            Some(ty) if !request.no_schema => Some(self.schema(&request.schema, ty)?),
            _ => None,
        };
        let oracle = CorpusOracle::new(self.corpus());
        let blocks = inputs
            .into_iter()
            .map(|input| {
                let from_label = match (&fixed, &input.label) {
                    (None, Some(label)) if !request.no_schema => self.schema_for_label(label),
                    _ => None,
                };
                let schema = fixed.as_ref().or(from_label.as_ref());
                let decoded = decode(&input.bytes, schema.map(|s| (&s.set, s.set.root)), &oracle);
                DecodedBlock {
                    encoding: input.encoding,
                    label: input.label,
                    root: schema.map(|s| s.name.clone()),
                    der: base64_encode(&input.bytes),
                    tlvs: decoded.tlvs,
                    findings: decoded.findings,
                }
            })
            .collect();
        Ok(DecodeResponse { blocks })
    }

    /// The backends the Codegen tab offers.
    pub fn backends() -> BackendsResponse {
        BackendsResponse {
            backends: backends().iter().map(|b| b.meta()).collect(),
        }
    }

    /// The code `request.backend` generates for the request's root type and everything it reaches.
    pub fn codegen(&self, request: &CodegenRequest) -> Result<CodegenResponse> {
        let target = backend(&request.backend).ok_or_else(|| {
            ServiceError::bad_request(format!("no backend `{}`", request.backend))
        })?;
        let schema = self.schema(&request.schema, &request.ty)?;
        let build = self.corpus().info().build.clone();
        let options = Options {
            glint_version: env!("CARGO_PKG_VERSION").to_owned(),
            data_build: (request.schema.document.is_none() && !build.is_empty()).then_some(build),
        };
        let output = generate(&schema.set, target, &options);
        Ok(CodegenResponse {
            backend: target.meta(),
            root: schema.name,
            files: output.files,
            findings: output.findings,
        })
    }
}
