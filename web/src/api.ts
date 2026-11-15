export type Severity = "error" | "warning" | "info";

export interface Label {
  file: number;
  span: { start: number; end: number };
  line_col: [number, number];
  section: string | null;
  text: string | null;
}

export interface Diagnostic {
  code: string;
  severity: Severity;
  message: string;
  primary: Label;
  secondary: Label[];
  notes: string[];
  help: string | null;
}

export interface ModuleRef {
  source: string;
  name: string;
  /** Present when the source has several modules of this name. */
  oid?: string;
}

/** `rfc4998#ERS`, with the OID when the source has several modules of that name. */
export function moduleLabel(ref: ModuleRef): string {
  return `${ref.source}#${ref.name}${ref.oid ? ` (${ref.oid})` : ""}`;
}

/** `5280` for the document id `rfc5280`; null for documents of other standards bodies. */
export function rfcNumber(document: string): number | null {
  const match = /^rfc(\d+)$/i.exec(document);
  return match ? Number(match[1]) : null;
}

/** `RFC 5280` for `rfc5280`; other document ids are titles already. */
export function documentTitle(document: string, edition?: string | null): string {
  const number = rfcNumber(document);
  const title = number === null ? document : `RFC ${number}`;
  return edition ? `${title} (${edition})` : title;
}

export interface FileInfo {
  id: number;
  name: string;
  kind: "document" | "corpus";
  module?: ModuleRef;
}

export interface BlockInfo {
  kind: "module" | "module-list" | "fragment";
  section: string | null;
  title: string | null;
  start: number;
  end: number;
  line: number;
  column: number;
}

export interface ImportInfo {
  module: string;
  resolved_to: string | null;
  target: ModuleRef | null;
  symbols: string[];
}

export interface ModuleInfo {
  name: string;
  oid: string | null;
  section: string | null;
  start: number;
  end: number;
  imports: ImportInfo[];
}

export type FormatName = "xml" | "markdown" | "text" | "bare";

/**
 * Lines `start_line..end_line` (1-based, inclusive) of the corrected text. A hunk that only removed lines has
 * `end_line === start_line - 1`: the empty place before line `start_line`.
 */
export interface HunkLines {
  start_line: number;
  end_line: number;
}

/** An RFC erratum whose patch the corpus applied to the document's text: for an RFC with two views, the patch of the
 * view checked (its text patch, or its XML patch) and its lines in that view. */
export interface AppliedErratum {
  id: number;
  /** Who decided to apply it: `human` or `claude`. */
  by: string;
  /** What the patch changes. */
  note: string;
  /** A unified diff of the document's source. */
  patch: string;
  /** Per hunk of the patch, where its new side is in the corrected text. */
  hunks: HunkLines[];
}

/** Why an RFC's errata were not applied: its text is the published one. */
export interface ErrataFailure {
  errata_id: number;
  reason: string;
}

/** Findings about an RFC's XML markup, counted by severity. */
export interface MarkupFindings {
  errors: number;
  warnings: number;
  infos: number;
}

export interface CheckResponse {
  schema_version: number;
  /** The version of Glint that checked, and the build of the data it checked against. */
  glint_version: string;
  data_build: string;
  document: {
    name: string;
    format: FormatName;
    /** The text, for a document checked by name. */
    content?: string;
    /** For a document checked by name: the errata applied to its text, in the order they were applied. */
    errata?: AppliedErratum[];
    errata_failure?: ErrataFailure;
    /**
     * For an RFC or draft whose source is RFCXML and that the corpus has both as its text and as its XML (an RFC's
     * corrected by its errata), checked by name: the URL of the published XML (the RFC Editor's for an RFC, the IETF
     * archive's for a draft). The text's ASN.1 blocks are the ones the XML marks, not the ones the text heuristics would
     * find.
     */
    xml_source?: string;
    /** For such a document: the format of its other view, which `checkDocument` with that `format` opens. */
    other_format?: "text" | "xml";
    /**
     * For the text of such a document: the findings about the RFCXML marking of its ASN.1 (`W202`, `W203`, ...) the check
     * kept out of `diagnostics`, by severity. The XML view shows them on its elements; the text view counts them with
     * its own.
     */
    markup_findings?: MarkupFindings;
  };
  files: FileInfo[];
  blocks: BlockInfo[];
  modules: ModuleInfo[];
  diagnostics: Diagnostic[];
}

export interface ModuleListItem {
  name: string;
  oid: string | null;
  /** The standards body: `rfc`, `itu-t`, `etsi`. */
  body: string;
  /** The document within the body: `rfc5280`, `X.509`. */
  document: string;
  edition: string | null;
  /** Where the module's text was downloaded from, for modules published as files of their own. */
  url: string | null;
  section: string | null;
  status: "clean" | "warnings" | "errors" | "unparseable";
  obsoleted_by: number[];
}

export interface ModuleDetail extends ModuleListItem {
  rfc_status: string | null;
  syntax_era: string;
  source_kind: { kind: string; [key: string]: unknown };
  sha256: string;
  text: string;
  imports: ImportInfo[];
}

export interface OidAssignment {
  oid: string;
  name: string;
  module: string;
  document: string;
}

export interface LabelEvidence {
  label: string;
  documents: string[];
}

export interface OidResponse {
  oid: string;
  parent: string | null;
  module: ModuleRef | null;
  /** The `name(number)` labels documents write for this arc. */
  labels: LabelEvidence[];
  /** Where the names written for the arcs disagree with the labels documents write for them. */
  label_notes: string[];
  /** Why the OID is on the allowlist of wrong but deployed OIDs, when it is. */
  allowlisted: string | null;
  assignments: OidAssignment[];
  children: OidAssignment[];
  truncated: boolean;
}

export interface ModulePage {
  total: number;
  offset: number;
  limit: number;
  modules: ModuleListItem[];
}

export interface CorpusResponse {
  glint_version: string;
  schema: number;
  build: string;
  notices: string[];
  modules: number;
  documents: number;
  by_status: Record<string, number>;
  oid_assignments: number;
  oid_nodes: number;
}

export interface CompletionItem {
  label: string;
  type: string;
  detail: string | null;
  apply?: string;
}

/** Completions for the word that starts at `from` (a UTF-16 position). */
export interface CompletionResponse {
  from: number;
  items: CompletionItem[];
}

export type InputFormat = "auto" | "pem" | "base64" | "der";

/** A document whose modules a decode can take its type from: the one open in the checker. */
export interface UserDocument {
  content: string;
  name?: string;
  format?: FormatName;
}

/** Where a decode's root type comes from: a corpus module, or a module of `document`, named by `module.name`. */
export interface SchemaSpec {
  module?: ModuleRef;
  document?: UserDocument;
}

export interface DecodeRequest extends SchemaSpec {
  input?: InputFormat;
  /** The root type; without it, each PEM block's label chooses one. */
  type?: string;
  no_schema?: boolean;
}

export interface DecodeFinding {
  code: string;
  severity: Severity;
  message: string;
}

export interface OidName {
  name: string;
  module: string;
  document?: string;
}

export type Rendered =
  | { kind: "boolean"; value: boolean }
  | { kind: "integer"; decimal: string; hex?: string; name?: string }
  | { kind: "enumerated"; decimal: string; name?: string }
  | { kind: "null" }
  | { kind: "bit-string"; unused: number; hex: string; named?: string[] }
  | { kind: "octet-string"; hex: string }
  | { kind: "oid"; dotted: string; names?: OidName[] }
  | { kind: "relative-oid"; dotted: string }
  | { kind: "time"; text: string; normalized?: string }
  | { kind: "string"; text: string }
  | { kind: "real"; text: string }
  | { kind: "hex"; hex: string };

export type TagClass = "universal" | "application" | "context" | "private";

/** One TLV of a decode (Codegen and Decoder Design, Decoder: Output). Offsets are into the block's octets. */
export interface Tlv {
  offset: number;
  header_len: number;
  len: { form: "definite"; octets: number } | { form: "indefinite" };
  /** The end of the TLV, end-of-contents octets included. */
  end: number;
  /** Absent for octets that do not start a TLV. */
  tag?: { class: TagClass; number: number; constructed: boolean };
  tagging?: "explicit" | "implicit";
  field?: string;
  type_name?: string;
  value?: Rendered;
  children?: Tlv[];
  findings?: DecodeFinding[];
  schema: { state: "matched" | "unmatched" } | { state: "hinted"; from: string };
}

export interface DecodedBlock {
  encoding: "pem" | "base64" | "der";
  label?: string;
  /** `doc#Module.Type` or `Module.Type`; absent for a decode without a schema. */
  root?: string;
  /** The octets, base64. */
  der: string;
  tlvs: Tlv[];
  findings?: DecodeFinding[];
}

export interface DecodeResponse {
  blocks: DecodedBlock[];
}

export interface TypesResponse {
  modules: { name: string; types: string[] }[];
}

export type Confidence = "label" | "decoded" | "unknown";

export interface ArtifactQuery {
  document?: string;
  label?: string;
  type?: string;
  confidence?: Confidence;
  errors?: boolean;
}

/** A PEM example of an RFC or draft, without its octets (Codegen and Decoder Design, Example artifacts). */
export interface ArtifactRow {
  id: string;
  document: string;
  section?: string;
  appendix?: boolean;
  title?: string;
  caption?: string;
  line: number;
  label: string;
  /** The identified type; for `unknown`, the closest candidate. */
  root?: string;
  confidence: Confidence;
  findings?: string[];
  /** The first error of the decode as `root`: with the decoder correct, the example has errors. */
  first_error?: string;
  obsoleted?: boolean;
  size: number;
}

export interface BrokenArtifact {
  id: string;
  document: string;
  section?: string;
  appendix?: boolean;
  line: number;
  label: string;
  message: string;
}

export interface ArtifactList {
  build: string;
  artifacts: ArtifactRow[];
  broken: BrokenArtifact[];
  labels: string[];
}

export interface ArtifactDetail extends ArtifactRow {
  /** The octets, base64. */
  der: string;
  canonical_label?: string;
  /** What a download gives: PEM with the canonical label, or base64 lines for a label the label map does not know. */
  text: string;
}

export interface CheckOptions {
  snippets?: boolean;
  pins?: Record<string, string>;
  severity?: Record<string, "off" | "error" | "warning" | "info">;
}

export class ApiError extends Error {
  constructor(
    readonly code: string,
    message: string,
  ) {
    super(message);
  }
}

/** What Glint is asked; the worker answers each with the JSON of the matching method of `glint-wasm`. */
export type Method =
  | "check"
  | "checkDocument"
  | "corpus"
  | "modules"
  | "module"
  | "oid"
  | "complete"
  | "decode"
  | "types"
  | "artifacts"
  | "artifact";

/** Where requests go: the Glint worker on the page, or a stand-in in tests. */
export interface Backend {
  call(method: Method, args: unknown[], signal?: AbortSignal): Promise<unknown>;
}

/** How far the data has got: the corpus, then the texts of the documents that can be checked by name. */
export type DataState =
  | { corpus: "loading" }
  | { corpus: "failed"; error: string }
  | { corpus: "ready"; info: CorpusResponse; documents: "loading" | "ready" | "failed" };

let backend: Backend | null = null;
let dataState: DataState = { corpus: "loading" };
const dataListeners = new Set<(state: DataState) => void>();

function setDataState(state: DataState): void {
  dataState = state;
  for (const listener of dataListeners) listener(state);
}

/** Calls `listener` with the state of the data now and whenever it changes; returns a function that stops it. */
export function onData(listener: (state: DataState) => void): () => void {
  dataListeners.add(listener);
  listener(dataState);
  return () => dataListeners.delete(listener);
}

/** Replaces the backend (tests); `null` goes back to the worker. */
export function setBackend(replacement: Backend | null): void {
  backend = replacement;
}

interface WorkerAnswer {
  id: number;
  ok: boolean;
  value?: string;
  error?: { code: string; message: string };
}

interface WorkerEvent {
  event: "corpus" | "documents";
  ok: boolean;
  info?: CorpusResponse;
  error?: string;
}

function workerBackend(): Backend {
  const worker = new Worker(new URL("./worker.ts", import.meta.url), { type: "module" });
  const pending = new Map<number, { resolve: (value: unknown) => void; reject: (error: Error) => void }>();
  let nextId = 1;
  worker.addEventListener("message", (message: MessageEvent<WorkerAnswer | WorkerEvent>) => {
    const data = message.data;
    if ("event" in data) {
      if (data.event === "corpus") {
        setDataState(data.ok && data.info ? { corpus: "ready", info: data.info, documents: "loading" } : { corpus: "failed", error: data.error ?? "unknown error" });
      } else if (dataState.corpus === "ready") {
        setDataState({ ...dataState, documents: data.ok ? "ready" : "failed" });
      }
      return;
    }
    const waiting = pending.get(data.id);
    if (!waiting) return;
    pending.delete(data.id);
    if (data.ok) waiting.resolve(JSON.parse(data.value ?? "null"));
    else waiting.reject(new ApiError(data.error?.code ?? "error", data.error?.message ?? "unknown error"));
  });
  worker.addEventListener("error", (event) => {
    const error = new ApiError("worker", `Glint could not start: ${event.message || "the worker failed"}`);
    for (const waiting of pending.values()) waiting.reject(error);
    pending.clear();
    setDataState({ corpus: "failed", error: error.message });
  });
  worker.postMessage({ type: "start", base: new URL(".", window.location.href).href });
  return {
    call(method, args, signal) {
      const id = nextId++;
      return new Promise((resolve, reject) => {
        // The worker finishes what it started; an aborted call only stops waiting for it.
        signal?.addEventListener("abort", () => {
          if (pending.delete(id)) reject(new DOMException("aborted", "AbortError"));
        });
        pending.set(id, { resolve, reject });
        worker.postMessage({ type: "call", id, method, args });
      });
    },
  };
}

function call<T>(method: Method, args: unknown[], signal?: AbortSignal): Promise<T> {
  backend ??= workerBackend();
  if (signal?.aborted) return Promise.reject(new DOMException("aborted", "AbortError"));
  return backend.call(method, args, signal) as Promise<T>;
}

export const api = {
  check(
    content: string,
    name: string,
    format: FormatName | null,
    options: CheckOptions,
    signal?: AbortSignal,
  ) {
    const request: Record<string, unknown> = { name, content, options };
    if (format) request.format = format;
    return call<CheckResponse>("check", [request], signal);
  },
  /**
   * Checks a draft or RFC by name, from the texts in the data; `format: "xml"` opens the RFCXML of one the data has as
   * its text (an RFC's corrected by its errata).
   */
  checkDocument(
    target: { draft: string; rev?: string; format?: "text" | "xml" } | { rfc: number; format?: "text" | "xml" },
    options: CheckOptions,
    signal?: AbortSignal,
  ) {
    return call<CheckResponse>("checkDocument", [{ ...target, options }], signal);
  },
  corpus: () => call<CorpusResponse>("corpus", []),
  modules: (q: string, offset: number, limit: number, signal?: AbortSignal) => call<ModulePage>("modules", [q, offset, limit], signal),
  module: (source: string, name: string, oid?: string) => call<ModuleDetail>("module", [source, name, oid ?? null]),
  /** `names` are the names written for the arcs, with an empty string for a bare number. */
  oid: (dotted: string, names: string[] = []) => call<OidResponse>("oid", [dotted, names]),
  complete: (text: string, name: string | null, format: FormatName | null, position: number, signal?: AbortSignal) =>
    call<CompletionResponse>("complete", [text, name, format, position], signal),
  /** Decodes `bytes`: PEM or base64 text (UTF-8), or DER. */
  decode: (request: DecodeRequest, bytes: Uint8Array, signal?: AbortSignal) => call<DecodeResponse>("decode", [request, bytes], signal),
  types: (spec: SchemaSpec, signal?: AbortSignal) => call<TypesResponse>("types", [spec], signal),
  /** The example artifacts; the first call fetches `artifacts.json`. */
  artifacts: (query: ArtifactQuery, signal?: AbortSignal) => call<ArtifactList>("artifacts", [query], signal),
  artifact: (id: string) => call<ArtifactDetail>("artifact", [id]),
};
