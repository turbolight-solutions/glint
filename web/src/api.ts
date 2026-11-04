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
  corpus_version: string;
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

export interface OidResponse {
  oid: string;
  parent: string | null;
  module: ModuleRef | null;
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
  version: string;
  notices: string[];
  modules: number;
  by_status: Record<string, number>;
  oid_assignments: number;
}

export interface CheckOptions {
  snippets?: boolean;
  pins?: Record<string, string>;
  severity?: Record<string, "off" | "error" | "warning" | "info">;
}

export class ApiError extends Error {
  constructor(
    readonly status: number,
    readonly code: string,
    message: string,
  ) {
    super(message);
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  let response: Response;
  try {
    response = await fetch(`/api/v1${path}`, init);
  } catch (error) {
    if (error instanceof DOMException && error.name === "AbortError") throw error;
    throw new ApiError(0, "network", "Cannot reach the server.");
  }
  if (!response.ok) {
    let code = "error";
    let message = `${response.status} ${response.statusText}`;
    try {
      const body = (await response.json()) as { error?: { code?: string; message?: string } };
      code = body.error?.code ?? code;
      message = body.error?.message ?? message;
    } catch {
      // the body was not JSON; keep the status line
    }
    throw new ApiError(response.status, code, message);
  }
  return (await response.json()) as T;
}

function post<T>(path: string, body: unknown, signal?: AbortSignal): Promise<T> {
  return request<T>(path, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
    signal,
  });
}

export const api = {
  check(
    content: string,
    name: string,
    format: FormatName | null,
    options: CheckOptions,
    signal?: AbortSignal,
  ) {
    const document: Record<string, unknown> = { name, content };
    if (format) document.format = format;
    return post<CheckResponse>("/check", { document, options }, signal);
  },
  checkDraft(target: { draft: string; rev?: string } | { rfc: number }, options: CheckOptions, signal?: AbortSignal) {
    return post<CheckResponse>("/check/draft", { ...target, options }, signal);
  },
  corpus: () => request<CorpusResponse>("/corpus"),
  modules: (q: string, offset: number, limit: number, signal?: AbortSignal) =>
    request<ModulePage>(`/modules?q=${encodeURIComponent(q)}&offset=${offset}&limit=${limit}`, { signal }),
  module: (source: string, name: string, oid?: string) =>
    request<ModuleDetail>(`/modules/${encodeURIComponent(source)}/${encodeURIComponent(name)}${oid ? `?oid=${encodeURIComponent(oid)}` : ""}`),
  oid: (dotted: string) => request<OidResponse>(`/oids/${encodeURIComponent(dotted)}`),
};
