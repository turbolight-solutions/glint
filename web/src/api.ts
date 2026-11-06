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

export interface CheckResponse {
  schema_version: number;
  /** The version of Glint that checked, and the build of the data it checked against. */
  glint_version: string;
  data_build: string;
  document: { name: string; format: FormatName; content?: string };
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
export type Method = "check" | "checkDocument" | "corpus" | "modules" | "module" | "oid" | "complete";

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
  /** Checks a draft or RFC by name, from the texts in the data. */
  checkDocument(target: { draft: string; rev?: string } | { rfc: number }, options: CheckOptions, signal?: AbortSignal) {
    return call<CheckResponse>("checkDocument", [{ ...target, options }], signal);
  },
  corpus: () => call<CorpusResponse>("corpus", []),
  modules: (q: string, offset: number, limit: number, signal?: AbortSignal) => call<ModulePage>("modules", [q, offset, limit], signal),
  module: (source: string, name: string, oid?: string) => call<ModuleDetail>("module", [source, name, oid ?? null]),
  /** `names` are the names written for the arcs, with an empty string for a bare number. */
  oid: (dotted: string, names: string[] = []) => call<OidResponse>("oid", [dotted, names]),
  complete: (text: string, name: string | null, format: FormatName | null, position: number, signal?: AbortSignal) =>
    call<CompletionResponse>("complete", [text, name, format, position], signal),
};
