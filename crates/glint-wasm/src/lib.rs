//! Glint for the browser. The page loads the code (this module) and the data (`corpus.json`, `documents.bin`)
//! separately, and every answer is JSON in the shapes of the page's `api.ts`. Nothing here does I/O.

mod complete;
mod service;

pub use complete::{Completion, Item, byte_to_utf16, complete, utf16_to_byte};
pub use service::{
    CheckRequest, DocumentRequest, FormatName, GLINT_VERSION, Service, ServiceError,
};

use serde::Serialize;
use wasm_bindgen::prelude::*;

/// Errors cross to JavaScript as JSON `{ "code", "message" }` in the message of an `Error`.
fn js_error(error: &ServiceError) -> JsError {
    JsError::new(&serde_json::to_string(error).unwrap_or_else(|_| error.message.clone()))
}

fn json<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_owned())
}

fn parse<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, JsError> {
    serde_json::from_str(text).map_err(|e| {
        js_error(&ServiceError {
            code: "bad_request",
            message: e.to_string(),
        })
    })
}

#[wasm_bindgen]
pub struct Glint {
    service: Service,
}

#[wasm_bindgen]
impl Glint {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            service: Service::default(),
        }
    }

    /// The version of the code.
    pub fn version() -> String {
        GLINT_VERSION.to_owned()
    }

    /// The data schema the code reads.
    pub fn schema() -> u32 {
        glint_corpus::SCHEMA
    }

    /// Loads `corpus.json`; answers with the corpus summary.
    #[wasm_bindgen(js_name = loadCorpus)]
    pub fn load_corpus(&mut self, bytes: &[u8]) -> Result<String, JsError> {
        self.service
            .load_corpus(bytes)
            .map(|info| json(&info))
            .map_err(|e| js_error(&e))
    }

    /// Loads `documents.bin`, which must be of the corpus's build.
    #[wasm_bindgen(js_name = loadDocuments)]
    pub fn load_documents(&mut self, bytes: Vec<u8>) -> Result<(), JsError> {
        self.service.load_documents(bytes).map_err(|e| js_error(&e))
    }

    #[wasm_bindgen(js_name = hasDocuments)]
    pub fn has_documents(&self) -> bool {
        self.service.has_documents()
    }

    pub fn corpus(&self) -> String {
        json(&self.service.corpus_info())
    }

    /// `request` is `{ content, name?, format?, options? }`.
    pub fn check(&self, request: &str) -> Result<String, JsError> {
        let request: CheckRequest = parse(request)?;
        self.service
            .check(request)
            .map(|r| json(&r))
            .map_err(|e| js_error(&e))
    }

    /// `request` is `{ draft, rev? }` or `{ rfc }`, with `options?`.
    #[wasm_bindgen(js_name = checkDocument)]
    pub fn check_document(&self, request: &str) -> Result<String, JsError> {
        let request: DocumentRequest = parse(request)?;
        self.service
            .check_document(request)
            .map(|r| json(&r))
            .map_err(|e| js_error(&e))
    }

    pub fn modules(&self, query: &str, offset: usize, limit: usize) -> String {
        json(&self.service.modules(query, offset, Some(limit)))
    }

    pub fn module(&self, source: &str, name: &str, oid: Option<String>) -> Result<String, JsError> {
        self.service
            .module(source, name, oid.as_deref())
            .map(|r| json(&r))
            .map_err(|e| js_error(&e))
    }

    /// Looks up `dotted`; `names` are the names written for its arcs, with an empty string for a bare number.
    pub fn oid(&self, dotted: &str, names: Vec<String>) -> Result<String, JsError> {
        self.service
            .oid(dotted, &names)
            .map(|r| json(&r))
            .map_err(|e| js_error(&e))
    }

    /// Completions at the UTF-16 `position` of a document; `format` is as in `check`, or detected.
    pub fn complete(
        &self,
        text: &str,
        name: Option<String>,
        format: Option<String>,
        position: usize,
    ) -> String {
        let format = format.and_then(|f| serde_json::from_value(serde_json::Value::String(f)).ok());
        json(&complete(
            &self.service,
            text,
            name.as_deref(),
            format,
            position,
        ))
    }
}

impl Default for Glint {
    fn default() -> Self {
        Self::new()
    }
}
