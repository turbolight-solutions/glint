// The workbench (`#workbench`): decode PEM, base64 or DER as a type of a corpus module or of the checker's document,
// browse the PEM examples of the RFCs and drafts, and generate code for a type (Codegen and Decoder Design, Web:
// `#/workbench`).
import {
  ApiError,
  type ArtifactList,
  type ArtifactQuery,
  type ArtifactRow,
  type BackendMeta,
  type CodegenResponse,
  type Confidence,
  type DecodeFinding,
  type DecodeRequest,
  type DecodedBlock,
  type InputFormat,
  type ModuleListItem,
  type ModuleRef,
  type Tlv,
  type UserDocument,
  api,
  documentTitle,
} from "./api";
import type { Extension } from "@codemirror/state";
import {
  type ArtifactSort,
  type ArtifactSortKey,
  artifactType,
  formatArtifactSort,
  nextArtifactSort,
  parseArtifactSort,
  sortArtifacts,
} from "./artifactsort";
import { debounce, h, replace } from "./dom";
import { type EditorHandle, createEditor } from "./editor";
import { CHECKER_SOURCE, type WorkbenchRoute, type WorkbenchTab, hashForTarget, moduleHash, rootRoute, workbenchHash } from "./router";
import { byteOwners, flatten, formatSize, fromBase64, holdsContents, shortType, tagText, valueText } from "./tlv";
import { zip } from "./zip";

const DECODE_MS = 300;
const SEARCH_MS = 200;
const MAX_FILE_BYTES = 8_000_000;
/** The hex view shows this much of a large input. */
const HEX_SHOWN_BYTES = 64 * 1024;
const PAGE_SIZE = 50;

const DEFAULT_BACKEND = "rasn";

const ARTIFACT_COLUMNS: { key: ArtifactSortKey; label: string }[] = [
  { key: "example", label: "Example" },
  { key: "label", label: "Label" },
  { key: "type", label: "Type" },
  { key: "confidence", label: "Confidence" },
  { key: "size", label: "Size" },
  { key: "findings", label: "Findings" },
];

const SEVERITY_BADGE: Record<DecodeFinding["severity"], string> = {
  error: "text-bg-danger",
  warning: "text-bg-warning",
  info: "text-bg-info",
};

const CONFIDENCE_BADGE: Record<Confidence, { badge: string; title: string }> = {
  label: { badge: "text-bg-success", title: "Identified by its PEM label" },
  decoded: { badge: "text-bg-primary", title: "Identified by decoding it as the document's own types and the common roots" },
  unknown: { badge: "text-bg-secondary", title: "No type fits; shown with the closest candidate" },
};

export interface WorkbenchOptions {
  /** The document open in the checker, for decoding with its modules. */
  checkerDocument(): { title: string; document: UserDocument } | null;
}

export interface Workbench {
  show(route: WorkbenchRoute): void;
}

/** The module and type the picker holds; `source` is a corpus document or {@link CHECKER_SOURCE}. */
interface Picked {
  source: string;
  module: string;
  oid?: string;
  type: string;
}

function samePick(a: Picked | null, b: Picked | null): boolean {
  return a?.source === b?.source && a?.module === b?.module && a?.oid === b?.oid && a?.type === b?.type;
}

function describe(error: unknown): string {
  if (error instanceof ApiError && error.code === "artifacts_loading") return "The example artifacts are still loading; try again in a moment.";
  return error instanceof Error ? error.message : String(error);
}

const isAbort = (error: unknown): boolean => error instanceof DOMException && error.name === "AbortError";

function saveFile(name: string, data: BlobPart, type: string): void {
  const url = URL.createObjectURL(new Blob([data], { type }));
  const link = h("a", { href: url, download: name, hidden: true });
  document.body.appendChild(link);
  link.click();
  link.remove();
  setTimeout(() => URL.revokeObjectURL(url), 0);
}

function findingBadge(finding: DecodeFinding): HTMLElement {
  return h("span", { class: `badge ${SEVERITY_BADGE[finding.severity]}`, title: finding.message }, finding.code);
}

/** `Section 4.2` or `Appendix A`, for an artifact's place in its document. */
function placeOf(row: { section?: string; appendix?: boolean }): string {
  if (!row.section) return "";
  return `${row.appendix ? "Appendix" : "Section"} ${row.section}`;
}

export function createWorkbench(root: HTMLElement, options: WorkbenchOptions): Workbench {
  root.className = "view workbench";
  let tab: WorkbenchTab = "decode";
  let picked: Picked | null = null;
  /** The artifact the input holds, until the input is edited. */
  let artifactId: string | null = null;
  /** A dropped or opened file that is not text: DER. */
  let file: { name: string; bytes: Uint8Array } | null = null;
  let decodeController: AbortController | null = null;
  let decodeSequence = 0;
  let blocks: DecodedBlock[] = [];
  let blockIndex = 0;
  /** The artifacts filters, and the document filter the route last gave. */
  let artifactsLoaded = false;
  let artifactsController: AbortController | null = null;
  let artifactList: ArtifactList | null = null;
  let artifactOffset = 0;
  let artifactSort: ArtifactSort | null = null;
  /** The artifacts the filters leave, in the column sort's order. */
  let sortedArtifacts: ArtifactRow[] = [];

  // Navigation -----------------------------------------------------------------------------

  const decodeLink = h("a", { class: "nav-link", href: "#workbench" }, "Decode");
  const artifactsLink = h("a", { class: "nav-link", href: "#workbench?tab=artifacts" }, "Artifacts");
  const codegenLink = h("a", { class: "nav-link", href: "#workbench?tab=codegen" }, "Codegen");
  const nav = h(
    "ul",
    { class: "nav nav-tabs px-3 pt-2 flex-shrink-0" },
    h("li", { class: "nav-item" }, decodeLink),
    h("li", { class: "nav-item" }, artifactsLink),
    h("li", { class: "nav-item" }, codegenLink),
  );

  function currentRoute(): Omit<WorkbenchRoute, "view"> {
    const route: Omit<WorkbenchRoute, "view"> = { tab };
    if (picked) {
      route.source = picked.source;
      route.module = picked.module;
      if (picked.oid) route.moduleOid = picked.oid;
      if (picked.type) route.type = picked.type;
    }
    if (artifactId) route.artifact = artifactId;
    const document = documentFilter.value.trim();
    if (document) route.document = document;
    if (artifactSort) route.sort = formatArtifactSort(artifactSort);
    if (backendSelect.value && backendSelect.value !== DEFAULT_BACKEND) route.backend = backendSelect.value;
    return route;
  }

  /** Writes the state into the hash without a `hashchange`, so links to the current state can be shared. */
  function syncHash(): void {
    const route = currentRoute();
    decodeLink.setAttribute("href", workbenchHash({ ...route, tab: "decode" }));
    artifactsLink.setAttribute("href", workbenchHash({ ...route, tab: "artifacts" }));
    codegenLink.setAttribute("href", workbenchHash({ ...route, tab: "codegen" }));
    const hash = workbenchHash(route);
    if (window.location.hash !== hash) history.replaceState(null, "", hash);
  }

  // Picker -----------------------------------------------------------------------------------

  const sourceSelect = h(
    "select",
    { class: "form-select form-select-sm w-auto", "aria-label": "Where the module comes from" },
    h("option", { value: "corpus" }, "Corpus module"),
    h("option", { value: CHECKER_SOURCE }, "Checker document"),
  );
  const moduleList = h("datalist", { id: "workbench-modules" });
  const moduleInput = h("input", {
    class: "form-control form-control-sm",
    type: "search",
    list: "workbench-modules",
    placeholder: "Module: rfc5912#PKIX1Explicit-2009",
    "aria-label": "Module",
    autocomplete: "off",
  });
  const typeList = h("datalist", { id: "workbench-types" });
  const typeInput = h("input", {
    class: "form-control form-control-sm",
    type: "search",
    list: "workbench-types",
    placeholder: "Type: Certificate",
    "aria-label": "Type",
    autocomplete: "off",
  });
  const noSchema = h("input", { class: "form-check-input", type: "checkbox", id: "workbench-no-schema" });
  const pickerNote = h("div", { class: "small text-secondary" });
  /** The corpus modules the module list offers, by the text an option puts in the input. */
  let moduleOptions = new Map<string, ModuleRef>();
  let searchController: AbortController | null = null;
  let typesController: AbortController | null = null;

  /** `rfc5912#PKIX1Explicit-2009`, with the OID when the list holds two modules of one name in one document. */
  function moduleText(ref: ModuleRef): string {
    return ref.oid ? `${ref.source}#${ref.name} (${ref.oid})` : `${ref.source}#${ref.name}`;
  }

  function fillModuleList(refs: ModuleRef[]): void {
    moduleOptions = new Map();
    const options = refs.map((ref) => {
      const text = moduleText(ref);
      moduleOptions.set(text, ref);
      return h("option", { value: text });
    });
    replace(moduleList, ...options);
  }

  const searchModules = debounce(async (query: string) => {
    searchController?.abort();
    const run = new AbortController();
    searchController = run;
    try {
      const page = await api.modules(query, 0, 30, run.signal);
      if (run.signal.aborted) return;
      const seen = new Map<string, number>();
      for (const item of page.modules) seen.set(`${item.document}#${item.name}`, (seen.get(`${item.document}#${item.name}`) ?? 0) + 1);
      fillModuleList(
        page.modules.map((item: ModuleListItem) => {
          const ref: ModuleRef = { source: item.document, name: item.name };
          if (item.oid && (seen.get(`${item.document}#${item.name}`) ?? 0) > 1) ref.oid = item.oid;
          return ref;
        }),
      );
    } catch (error) {
      if (!isAbort(error)) console.error("glint: module search failed:", error);
    }
  }, SEARCH_MS);

  /** The module the input names: an option of the list, or any `source#Module` for a corpus module. */
  function moduleFromInput(): ModuleRef | null {
    const text = moduleInput.value.trim();
    if (!text) return null;
    const option = moduleOptions.get(text);
    if (option) return option;
    if (sourceSelect.value === CHECKER_SOURCE) return { source: CHECKER_SOURCE, name: text };
    const match = /^([^#\s]+)#(\S+)(?:\s+\(([\d.]+)\))?$/.exec(text);
    return match?.[1] && match[2] ? { source: match[1], name: match[2], ...(match[3] ? { oid: match[3] } : {}) } : null;
  }

  function checkerDocument(): { title: string; document: UserDocument } | null {
    return options.checkerDocument();
  }

  /** Offers the modules of the checker's document. */
  async function offerCheckerModules(): Promise<void> {
    const open = checkerDocument();
    if (!open) {
      fillModuleList([]);
      pickerNote.textContent = "Open or paste a document in the checker to decode with its modules.";
      return;
    }
    try {
      const answer = await api.types({ document: open.document });
      fillModuleList(answer.modules.map((m) => ({ source: CHECKER_SOURCE, name: m.name })));
      if (answer.modules.length === 0) pickerNote.textContent = `${open.title} has no ASN.1 modules.`;
      else if (answer.modules.length === 1 && !moduleInput.value && answer.modules[0]) {
        moduleInput.value = answer.modules[0].name;
        pickModule();
      }
    } catch (error) {
      pickerNote.textContent = describe(error);
    }
  }

  /** The schema part of a decode or types request for a module. */
  function schemaOf(module: ModuleRef): { module: ModuleRef; document?: UserDocument } | string {
    if (module.source !== CHECKER_SOURCE) return { module };
    const open = checkerDocument();
    return open ? { module, document: open.document } : "Open or paste a document in the checker to decode with its modules.";
  }

  async function loadTypes(module: ModuleRef | null): Promise<void> {
    typesController?.abort();
    replace(typeList);
    if (!module) return;
    const schema = schemaOf(module);
    if (typeof schema === "string") return;
    const run = new AbortController();
    typesController = run;
    try {
      const answer = await api.types(schema, run.signal);
      if (run.signal.aborted) return;
      const types = answer.modules.find((m) => m.name === module.name)?.types ?? [];
      replace(typeList, ...types.map((name) => h("option", { value: name })));
    } catch (error) {
      if (!isAbort(error)) pickerNote.textContent = describe(error);
    }
  }

  function describePick(): void {
    if (tab === "codegen") {
      if (picked?.type) {
        const where = picked.source === CHECKER_SOURCE ? `${picked.module} of the checker document` : `${picked.source}#${picked.module}`;
        pickerNote.textContent = `Generating code for ${picked.type} from ${where}, and every type it reaches.`;
      } else pickerNote.textContent = "Pick a module and a type to generate code for it and every type it reaches.";
      return;
    }
    if (noSchema.checked) pickerNote.textContent = "Decoding without a schema.";
    else if (picked?.type) {
      const where = picked.source === CHECKER_SOURCE ? `${picked.module} of the checker document` : `${picked.source}#${picked.module}`;
      pickerNote.textContent = `Decoding as ${picked.type} from ${where}.`;
    } else pickerNote.textContent = "No type picked: a PEM label chooses one when the corpus has it; anything else is decoded without a schema.";
  }

  /** Takes the module input as the picked module, keeping the type if the module did not change. */
  function pickModule(): void {
    const module = moduleFromInput();
    const type = typeInput.value.trim();
    const next: Picked | null = module ? { source: module.source, module: module.name, ...(module.oid ? { oid: module.oid } : {}), type } : null;
    const moduleChanged = next?.source !== picked?.source || next?.module !== picked?.module || next?.oid !== picked?.oid;
    if (moduleChanged) void loadTypes(module);
    if (samePick(next, picked)) return;
    picked = next;
    describePick();
    syncHash();
    void runDecode();
    if (tab === "codegen") void runCodegen();
  }

  function setPicker(next: Picked | null): void {
    picked = next;
    sourceSelect.value = next?.source === CHECKER_SOURCE ? CHECKER_SOURCE : "corpus";
    moduleInput.value = next ? (next.source === CHECKER_SOURCE ? next.module : moduleText({ source: next.source, name: next.module, oid: next.oid })) : "";
    typeInput.value = next?.type ?? "";
    void loadTypes(next ? { source: next.source, name: next.module, ...(next.oid ? { oid: next.oid } : {}) } : null);
    describePick();
  }

  sourceSelect.addEventListener("change", () => {
    moduleInput.value = "";
    typeInput.value = "";
    fillModuleList([]);
    if (sourceSelect.value === CHECKER_SOURCE) void offerCheckerModules();
    pickModule();
  });
  moduleInput.addEventListener("input", () => {
    if (moduleOptions.has(moduleInput.value)) {
      pickModule();
      return;
    }
    if (sourceSelect.value !== CHECKER_SOURCE) searchModules(moduleInput.value.trim());
  });
  moduleInput.addEventListener("focus", () => {
    if (sourceSelect.value === CHECKER_SOURCE) void offerCheckerModules();
    else if (moduleOptions.size === 0) searchModules(moduleInput.value.trim());
  });
  moduleInput.addEventListener("change", pickModule);
  typeInput.addEventListener("change", pickModule);
  typeInput.addEventListener("input", () => {
    if (Array.from(typeList.options).some((o) => o.value === typeInput.value)) pickModule();
  });
  noSchema.addEventListener("change", () => {
    describePick();
    void runDecode();
  });

  const noSchemaCheck = h("div", { class: "form-check mb-0" }, noSchema, h("label", { class: "form-check-label small", for: "workbench-no-schema" }, "No schema"));
  const picker = h(
    "div",
    { class: "d-flex flex-wrap align-items-center gap-2" },
    sourceSelect,
    h("div", { class: "workbench-module" }, moduleInput, moduleList),
    h("div", { class: "workbench-type" }, typeInput, typeList),
    noSchemaCheck,
  );
  /** The picker, above the Decode and Codegen tabs; an element of its own, since `d-flex` would override `hidden`. */
  const pickerBar = h("div", { class: "flex-shrink-0" }, h("div", { class: "d-flex flex-column gap-2 px-3 pt-3" }, picker, pickerNote));

  // Input ------------------------------------------------------------------------------------

  const textarea = h("textarea", {
    class: "form-control font-monospace small workbench-input",
    rows: 5,
    spellcheck: "false",
    placeholder: "Paste PEM or base64 here, or drop a PEM, base64 or DER file onto this page",
    "aria-label": "Input to decode",
  });
  const formatSelect = h(
    "select",
    { class: "form-select form-select-sm w-auto", "aria-label": "Input format" },
    h("option", { value: "auto" }, "Auto-detect"),
    h("option", { value: "pem" }, "PEM"),
    h("option", { value: "base64" }, "Base64"),
    h("option", { value: "der" }, "DER"),
  );
  const fileInput = h("input", { type: "file", hidden: true });
  const fileChip = h("span", { class: "small" });
  const blockSelect = h("select", { class: "form-select form-select-sm w-auto", "aria-label": "PEM block", hidden: true });
  const decodeStatus = h("span", { class: "small text-secondary", role: "status", "aria-live": "polite" });
  const decodeAlert = h("div");

  function setFile(next: { name: string; bytes: Uint8Array } | null): void {
    file = next;
    textarea.disabled = next !== null;
    replace(
      fileChip,
      next
        ? h(
            "span",
            { class: "badge text-bg-light border" },
            `${next.name} · ${formatSize(next.bytes.length)}`,
            h("button", { type: "button", class: "btn-close ms-2 workbench-chip-close", "aria-label": "Remove the file", onclick: () => clearInput() }),
          )
        : null,
    );
  }

  function clearInput(): void {
    setFile(null);
    textarea.value = "";
    artifactId = null;
    syncHash();
    void runDecode();
  }

  /** A file of text goes into the textarea, to be read as PEM or base64; anything else is kept as DER. */
  async function takeFile(picked: File): Promise<void> {
    if (picked.size > MAX_FILE_BYTES) {
      replace(decodeAlert, h("div", { class: "alert alert-danger py-1 small" }, `${picked.name} is larger than 8 MB.`));
      return;
    }
    const bytes = new Uint8Array(await picked.arrayBuffer());
    let text: string | null = null;
    try {
      text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
    } catch {
      text = null;
    }
    // Text files are PEM or base64; control characters other than whitespace mean DER.
    if (text !== null && !/[\u0000-\u0008\u000e-\u001f]/.test(text)) {
      setFile(null);
      textarea.value = text;
    } else {
      textarea.value = "";
      setFile({ name: picked.name, bytes });
    }
    artifactId = null;
    syncHash();
    void runDecode();
  }

  fileInput.addEventListener("change", () => {
    const chosen = fileInput.files?.[0];
    fileInput.value = "";
    if (chosen) void takeFile(chosen);
  });
  const decodeLater = debounce(() => void runDecode(), DECODE_MS);
  textarea.addEventListener("input", () => {
    if (artifactId) {
      artifactId = null;
      syncHash();
    }
    decodeLater();
  });
  formatSelect.addEventListener("change", () => void runDecode());
  blockSelect.addEventListener("change", () => {
    blockIndex = Number(blockSelect.value);
    renderBlock();
  });

  const inputArea = h(
    "div",
    { class: "d-flex flex-column gap-2" },
    textarea,
    h(
      "div",
      { class: "d-flex flex-wrap align-items-center gap-2" },
      h("button", { class: "btn btn-sm btn-outline-secondary", type: "button", title: "Open a PEM, base64 or DER file; you can also drop one onto this page", onclick: () => fileInput.click() }, "Open file…"),
      h("button", { class: "btn btn-sm btn-outline-secondary", type: "button", onclick: () => clearInput() }, "Clear"),
      formatSelect,
      fileInput,
      fileChip,
      blockSelect,
      decodeStatus,
    ),
  );

  // Output -----------------------------------------------------------------------------------

  const summary = h("div", { class: "small mb-2" });
  const treePane = h("div", { class: "workbench-tree font-monospace small" });
  const hexPane = h("div", { class: "workbench-hex font-monospace small" });
  const output = h(
    "div",
    { class: "workbench-output" },
    h("div", { class: "workbench-tree-wrap" }, summary, treePane),
    h("div", { class: "workbench-hex-wrap" }, hexPane),
  );

  /** The shown block: its TLVs depth first, the parent of each, the owner of each byte, and the cells of each byte. */
  let nodes: Tlv[] = [];
  let parents: number[] = [];
  /** The number of TLVs in the subtree of each, itself included. */
  let sizes: number[] = [];
  let owners: Int32Array = new Int32Array(0);
  let rows: HTMLElement[] = [];
  let hexCells: HTMLElement[] = [];
  let asciiCells: HTMLElement[] = [];
  let hovered = -1;
  let selected = -1;

  function markBytes(index: number, kind: "h" | "s", on: boolean): void {
    const tlv = nodes[index];
    if (!tlv) return;
    const contentStart = tlv.offset + tlv.header_len;
    const end = Math.min(tlv.end, hexCells.length);
    for (let i = Math.max(0, tlv.offset); i < end; i++) {
      const part = i < contentStart ? "hdr" : "body";
      hexCells[i]?.classList.toggle(`hx-${kind}-${part}`, on);
      asciiCells[i]?.classList.toggle(`hx-${kind}-${part}`, on);
    }
  }

  function hover(index: number): void {
    if (index === hovered) return;
    if (hovered >= 0) {
      markBytes(hovered, "h", false);
      rows[hovered]?.classList.remove("hovered");
    }
    hovered = index;
    if (index >= 0) {
      markBytes(index, "h", true);
      rows[index]?.classList.add("hovered");
    }
  }

  function select(index: number, from: "tree" | "hex"): void {
    if (selected >= 0) {
      markBytes(selected, "s", false);
      rows[selected]?.classList.remove("selected");
      rows[selected]?.setAttribute("aria-selected", "false");
    }
    selected = index;
    if (index < 0) return;
    markBytes(index, "s", true);
    const row = rows[index];
    row?.classList.add("selected");
    row?.setAttribute("aria-selected", "true");
    if (from === "hex") {
      // Open the TLVs around it, so its row can be seen.
      for (let p = parents[index] ?? -1; p >= 0; p = parents[p] ?? -1) rows[p]?.parentElement?.classList.remove("collapsed");
      row?.scrollIntoView({ block: "nearest" });
    } else {
      const first = hexCells[nodes[index]?.offset ?? 0];
      first?.scrollIntoView({ block: "nearest" });
    }
  }

  function schemaBadge(tlv: Tlv, parent: Tlv | undefined): HTMLElement | null {
    // A badge where the state starts, not on every TLV of its subtree.
    if (parent?.schema.state === tlv.schema.state) return null;
    if (tlv.schema.state === "hinted") {
      return h(
        "span",
        { class: "badge text-bg-warning-subtle border border-warning-subtle text-warning-emphasis", title: `Decoded as the type the OID hint table names, from ${tlv.schema.from}: a heuristic for a module without tables` },
        "~ hinted",
      );
    }
    if (tlv.schema.state === "unmatched" && parent !== undefined) {
      return h("span", { class: "badge text-bg-secondary", title: "Not of its type, or of no known type: decoded without a schema" }, "no schema");
    }
    return null;
  }

  function lengthText(tlv: Tlv): string {
    return tlv.len.form === "definite" ? String(tlv.len.octets) : "inf";
  }

  function renderNode(index: number): HTMLElement {
    const tlv = nodes[index] as Tlv;
    const parent = nodes[parents[index] ?? -1];
    const children = tlv.children ?? [];
    const value = valueText(tlv);
    const typeName = tlv.type_name;
    const toggle = children.length
      ? h("button", {
          type: "button",
          class: "tlv-toggle",
          "aria-label": "Collapse or expand",
          onclick: (event: Event) => {
            event.stopPropagation();
            node.classList.toggle("collapsed");
          },
        })
      : h("span", { class: "tlv-toggle-space" });
    const row = h(
      "div",
      {
        class: "tlv-row",
        role: "treeitem",
        tabindex: 0,
        "aria-selected": "false",
        title: `offset ${tlv.offset}, header ${tlv.header_len}, length ${lengthText(tlv)}`,
        onclick: () => select(index, "tree"),
        onkeydown: (event: Event) => {
          const key = (event as KeyboardEvent).key;
          if (key === "Enter" || key === " ") {
            event.preventDefault();
            select(index, "tree");
          }
        },
        onmouseenter: () => hover(index),
        onmouseleave: () => hover(-1),
      },
      toggle,
      h("span", { class: "tlv-offset text-secondary" }, String(tlv.offset)),
      h("span", { class: "tlv-tag" }, tagText(tlv)),
      tlv.field || typeName
        ? h(
            "span",
            { class: "tlv-name" },
            tlv.field ? h("span", { class: "tlv-field" }, `${tlv.schema.state === "hinted" ? "~" : ""}${tlv.field}`) : null,
            tlv.field && typeName ? ": " : null,
            typeName ? h("span", { class: "tlv-type", title: typeName }, shortType(typeName)) : null,
          )
        : null,
      value !== null ? h("span", { class: "tlv-value" }, value) : null,
      schemaBadge(tlv, parent),
      ...(tlv.findings ?? []).map(findingBadge),
    );
    rows[index] = row;
    const findings = (tlv.findings ?? []).map((f) => h("div", { class: `tlv-finding tlv-finding-${f.severity}` }, `${f.code} ${f.severity}: ${f.message}`));
    const node = h("div", { class: "tlv", role: "none" }, row, ...findings);
    if (children.length) {
      const group = h("div", { class: `tlv-children${holdsContents(tlv) ? " tlv-contained" : ""}`, role: "group" });
      let child = index + 1;
      for (let n = 0; n < children.length; n++) {
        group.appendChild(renderNode(child));
        child += sizes[child] ?? 1;
      }
      node.appendChild(group);
    }
    return node;
  }

  function renderHex(bytes: Uint8Array): void {
    hexCells = [];
    asciiCells = [];
    const shown = Math.min(bytes.length, HEX_SHOWN_BYTES);
    const fragment = document.createDocumentFragment();
    for (let start = 0; start < shown; start += 16) {
      const hexPart = h("span", { class: "hex-bytes" });
      const asciiPart = h("span", { class: "hex-ascii" });
      for (let i = start; i < Math.min(start + 16, shown); i++) {
        const byte = bytes[i] ?? 0;
        const cell = h("span", { "data-i": i }, byte.toString(16).padStart(2, "0"));
        const char = h("span", { "data-i": i }, byte >= 0x20 && byte < 0x7f ? String.fromCharCode(byte) : ".");
        hexCells.push(cell);
        asciiCells.push(char);
        hexPart.appendChild(cell);
        asciiPart.appendChild(char);
      }
      fragment.appendChild(h("div", { class: "hex-row" }, h("span", { class: "hex-offset text-secondary" }, start.toString(16).padStart(6, "0")), hexPart, asciiPart));
    }
    replace(hexPane, fragment, bytes.length > shown ? h("div", { class: "text-secondary p-2" }, `Showing the first ${formatSize(shown)} of ${formatSize(bytes.length)}.`) : null);
  }

  function byteAt(event: Event): number {
    const target = event.target as HTMLElement | null;
    const value = target?.getAttribute("data-i");
    return value === null || value === undefined ? -1 : Number(value);
  }

  hexPane.addEventListener("mouseover", (event) => {
    const i = byteAt(event);
    hover(i >= 0 ? (owners[i] ?? -1) : -1);
  });
  hexPane.addEventListener("mouseleave", () => hover(-1));
  hexPane.addEventListener("click", (event) => {
    const i = byteAt(event);
    if (i >= 0 && (owners[i] ?? -1) >= 0) select(owners[i] ?? -1, "hex");
  });

  function countFindings(block: DecodedBlock): { errors: number; warnings: number } {
    const all = [...flatten(block.tlvs).flatMap((t) => t.findings ?? []), ...(block.findings ?? [])];
    return { errors: all.filter((f) => f.severity === "error").length, warnings: all.filter((f) => f.severity === "warning").length };
  }

  function rootLink(rootType: string): Node {
    const route = rootRoute(rootType);
    return route?.source && route.module
      ? h("a", { href: moduleHash(route.source, route.module), title: "Open the module in the checker" }, rootType)
      : document.createTextNode(rootType);
  }

  function renderBlock(): void {
    hovered = -1;
    selected = -1;
    rows = [];
    const block = blocks[blockIndex];
    if (!block) {
      nodes = [];
      parents = [];
      owners = new Int32Array(0);
      replace(summary);
      replace(treePane, h("div", { class: "text-secondary p-2" }, "Paste or drop something to decode it."));
      replace(hexPane);
      return;
    }
    const bytes = fromBase64(block.der);
    nodes = flatten(block.tlvs);
    parents = [];
    sizes = [];
    const fill = (tlv: Tlv, parent: number): void => {
      const index = parents.length;
      parents.push(parent);
      sizes.push(1);
      for (const child of tlv.children ?? []) fill(child, index);
      sizes[index] = parents.length - index;
    };
    for (const tlv of block.tlvs) fill(tlv, -1);
    owners = byteOwners(nodes, bytes.length);
    const { errors, warnings } = countFindings(block);
    replace(
      summary,
      [block.label, block.encoding.toUpperCase(), formatSize(bytes.length)].filter(Boolean).join(" · "),
      " · ",
      block.root ? h("span", {}, "as ", rootLink(block.root)) : h("span", { class: "text-secondary" }, "without a schema"),
      " · ",
      h("span", { class: errors ? "text-danger" : warnings ? "text-warning-emphasis" : "text-success" }, `${errors} error${errors === 1 ? "" : "s"}, ${warnings} warning${warnings === 1 ? "" : "s"}`),
      ...(block.findings ?? []).map((f) => h("div", { class: `tlv-finding tlv-finding-${f.severity}` }, `${f.code} ${f.severity}: ${f.message}`)),
    );
    const tree = h("div", { role: "tree", "aria-label": "TLV tree" });
    for (let index = 0; index < nodes.length; index += sizes[index] ?? 1) tree.appendChild(renderNode(index));
    replace(treePane, tree);
    renderHex(bytes);
  }

  function renderBlocks(): void {
    if (blockIndex >= blocks.length) blockIndex = 0;
    blockSelect.hidden = blocks.length < 2;
    replace(
      blockSelect,
      ...blocks.map((block, i) =>
        h("option", { value: i, selected: i === blockIndex }, `Block ${i + 1} of ${blocks.length}${block.label ? `: ${block.label}` : ""}${block.root ? ` as ${shortType(block.root.split("#").pop() ?? block.root)}` : ""}`),
      ),
    );
    renderBlock();
  }

  function inputBytes(): Uint8Array {
    return file ? file.bytes : new TextEncoder().encode(textarea.value);
  }

  async function runDecode(): Promise<void> {
    decodeController?.abort();
    const sequence = ++decodeSequence;
    replace(decodeAlert);
    const bytes = inputBytes();
    if (bytes.length === 0 || (!file && textarea.value.trim() === "")) {
      blocks = [];
      decodeStatus.textContent = "";
      renderBlocks();
      return;
    }
    const request: DecodeRequest = { input: formatSelect.value as InputFormat };
    if (noSchema.checked) request.no_schema = true;
    else if (picked?.type) {
      const schema = schemaOf({ source: picked.source, name: picked.module, ...(picked.oid ? { oid: picked.oid } : {}) });
      if (typeof schema === "string") {
        replace(decodeAlert, h("div", { class: "alert alert-warning py-1 small" }, schema));
        return;
      }
      Object.assign(request, schema, { type: picked.type });
    }
    const run = new AbortController();
    decodeController = run;
    decodeStatus.textContent = "Decoding…";
    try {
      const answer = await api.decode(request, bytes, run.signal);
      if (sequence !== decodeSequence) return;
      blocks = answer.blocks;
      decodeStatus.textContent = blocks.length > 1 ? `${blocks.length} blocks` : "";
      renderBlocks();
    } catch (error) {
      if (isAbort(error) || sequence !== decodeSequence) return;
      decodeStatus.textContent = "";
      blocks = [];
      renderBlocks();
      replace(decodeAlert, h("div", { class: "alert alert-danger py-1 small", role: "alert" }, describe(error)));
    }
  }

  async function loadArtifact(id: string, route: WorkbenchRoute): Promise<void> {
    artifactId = id;
    decodeStatus.textContent = `Loading ${id}…`;
    try {
      const artifact = await api.artifact(id);
      if (artifactId !== id) return;
      setFile(null);
      textarea.value = artifact.text;
      formatSelect.value = "auto";
      // A link to an artifact alone decodes it as the type it was identified as.
      if (!route.module && artifact.root) {
        const fromRoot = rootRoute(artifact.root);
        if (fromRoot?.source && fromRoot.module && fromRoot.type) setPicker({ source: fromRoot.source, module: fromRoot.module, type: fromRoot.type });
      }
      syncHash();
      await runDecode();
    } catch (error) {
      if (artifactId !== id) return;
      artifactId = null;
      decodeStatus.textContent = "";
      replace(decodeAlert, h("div", { class: "alert alert-danger py-1 small", role: "alert" }, describe(error)));
      syncHash();
    }
  }

  const decodePane = h(
    "div",
    { class: "workbench-pane workbench-decode drop-zone", "data-drop-hint": "Drop a PEM, base64 or DER file to decode it" },
    h("div", { class: "d-flex flex-column gap-2 p-3 pb-2 flex-shrink-0" }, inputArea, decodeAlert),
    output,
  );
  let dragDepth = 0;
  decodePane.addEventListener("dragenter", (event) => {
    if (!event.dataTransfer?.types.includes("Files")) return;
    event.preventDefault();
    dragDepth++;
    decodePane.classList.add("dragging");
  });
  decodePane.addEventListener("dragover", (event) => {
    if (event.dataTransfer?.types.includes("Files")) event.preventDefault();
  });
  decodePane.addEventListener("dragleave", () => {
    dragDepth = Math.max(0, dragDepth - 1);
    if (dragDepth === 0) decodePane.classList.remove("dragging");
  });
  decodePane.addEventListener("drop", (event) => {
    event.preventDefault();
    dragDepth = 0;
    decodePane.classList.remove("dragging");
    const dropped = event.dataTransfer?.files[0];
    if (dropped) void takeFile(dropped);
  });

  // Artifacts --------------------------------------------------------------------------------

  const documentFilter = h("input", { class: "form-control form-control-sm", type: "search", placeholder: "Document: rfc9500", "aria-label": "Document" });
  const labelFilter = h("select", { class: "form-select form-select-sm w-auto", "aria-label": "PEM label" }, h("option", { value: "" }, "Any label"));
  const typeFilter = h("input", { class: "form-control form-control-sm", type: "search", placeholder: "Type contains: Certificate", "aria-label": "Identified type" });
  const confidenceFilter = h(
    "select",
    { class: "form-select form-select-sm w-auto", "aria-label": "Confidence" },
    h("option", { value: "" }, "Any confidence"),
    h("option", { value: "label" }, "By label"),
    h("option", { value: "decoded" }, "By decoding"),
    h("option", { value: "unknown" }, "Unknown"),
  );
  const errorsFilter = h("input", { class: "form-check-input", type: "checkbox", id: "workbench-errors" });
  const artifactsSummary = h("div", { class: "small text-secondary my-2" });
  const artifactsAlert = h("div");
  const artifactsTable = h("div");

  function query(): ArtifactQuery {
    const q: ArtifactQuery = {};
    if (documentFilter.value.trim()) q.document = documentFilter.value.trim();
    if (labelFilter.value) q.label = labelFilter.value;
    if (typeFilter.value.trim()) q.type = typeFilter.value.trim();
    if (confidenceFilter.value) q.confidence = confidenceFilter.value as Confidence;
    if (errorsFilter.checked) q.errors = true;
    return q;
  }

  async function runArtifacts(): Promise<void> {
    artifactsController?.abort();
    const run = new AbortController();
    artifactsController = run;
    artifactsLoaded = true;
    if (!artifactList) replace(artifactsTable, h("div", { class: "text-secondary p-2" }, "Loading the example artifacts…"));
    replace(artifactsAlert);
    try {
      const list = await api.artifacts(query(), run.signal);
      if (run.signal.aborted) return;
      artifactList = list;
      sortedArtifacts = sortArtifacts(list.artifacts, artifactSort);
      artifactOffset = 0;
      const chosen = labelFilter.value;
      replace(labelFilter, h("option", { value: "" }, "Any label"), ...list.labels.map((label) => h("option", { value: label, selected: label === chosen }, label)));
      renderArtifacts();
    } catch (error) {
      if (isAbort(error)) return;
      artifactsLoaded = false;
      replace(artifactsTable);
      replace(artifactsAlert, h("div", { class: "alert alert-danger py-1 small", role: "alert" }, describe(error)));
    }
  }

  const artifactsLater = debounce(() => {
    syncHash();
    void runArtifacts();
  }, SEARCH_MS);

  async function withArtifact(id: string, act: (artifact: Awaited<ReturnType<typeof api.artifact>>) => void | Promise<void>): Promise<void> {
    try {
      await act(await api.artifact(id));
    } catch (error) {
      replace(artifactsAlert, h("div", { class: "alert alert-danger py-1 small", role: "alert" }, describe(error)));
    }
  }

  function artifactActions(row: ArtifactRow): HTMLElement {
    const copy = h("button", { class: "btn btn-sm btn-outline-secondary", type: "button", title: "Copy the PEM (or base64) to the clipboard" }, "Copy");
    copy.addEventListener("click", () =>
      void withArtifact(row.id, async (artifact) => {
        await navigator.clipboard.writeText(artifact.text);
        copy.textContent = "Copied";
        setTimeout(() => (copy.textContent = "Copy"), 1500);
      }),
    );
    const route = row.root ? rootRoute(row.root) : null;
    const openHash = workbenchHash({ tab: "decode", ...(route ?? {}), artifact: row.id });
    return h(
      "div",
      { class: "btn-group btn-group-sm text-nowrap" },
      h(
        "button",
        {
          class: "btn btn-sm btn-outline-secondary",
          type: "button",
          title: "Download as PEM when the label is a known one, else as base64",
          onclick: () => void withArtifact(row.id, (artifact) => saveFile(`${row.id}.${artifact.canonical_label ? "pem" : "b64"}`, artifact.text, "text/plain")),
        },
        "Download",
      ),
      h(
        "button",
        {
          class: "btn btn-sm btn-outline-secondary",
          type: "button",
          title: "Download the DER octets",
          onclick: () => void withArtifact(row.id, (artifact) => saveFile(`${row.id}.der`, fromBase64(artifact.der) as Uint8Array<ArrayBuffer>, "application/octet-stream")),
        },
        "DER",
      ),
      copy,
      h("a", { class: "btn btn-sm btn-primary", href: openHash, title: "Decode it as the type it was identified as" }, "Open in Decode"),
    );
  }

  function documentLink(row: { document: string; section?: string; appendix?: boolean; line: number }): HTMLElement {
    const hash = hashForTarget(row.document);
    const title = documentTitle(row.document);
    const place = placeOf(row);
    return h(
      "div",
      {},
      hash ? h("a", { href: hash, title: "Open the document in the checker" }, title) : title,
      place ? h("span", { class: "text-secondary" }, ` · ${place}`) : null,
      h("span", { class: "text-secondary" }, ` · line ${row.line}`),
    );
  }

  function artifactRow(row: ArtifactRow): HTMLElement {
    const confidence = CONFIDENCE_BADGE[row.confidence];
    const typeName = artifactType(row);
    return h(
      "tr",
      {},
      h("td", {}, h("div", { class: "font-monospace" }, row.id), documentLink(row), row.caption || row.title ? h("div", { class: "small text-secondary" }, row.caption ?? row.title ?? "") : null),
      h("td", { class: "font-monospace small" }, row.label),
      h("td", {}, typeName ? h("span", { title: row.root ?? "" }, shortType(typeName)) : h("span", { class: "text-secondary" }, "—"), row.root ? h("div", { class: "small text-secondary" }, row.root.split("#")[0] ?? "") : null),
      h(
        "td",
        {},
        h("span", { class: `badge ${confidence.badge}`, title: row.confidence === "unknown" && row.first_error ? `${confidence.title}; its first mismatch: ${row.first_error}` : confidence.title }, row.confidence),
      ),
      h("td", { class: "text-nowrap" }, formatSize(row.size)),
      h(
        "td",
        {},
        h(
          "div",
          { class: "d-flex flex-wrap gap-1" },
          row.first_error && row.confidence !== "unknown" ? h("span", { class: "badge text-bg-danger", title: row.first_error }, "example has errors") : null,
          ...(row.findings ?? []).map((code) => h("span", { class: `badge ${code.startsWith("E") ? "text-bg-danger" : code.startsWith("W") ? "text-bg-warning" : "text-bg-info"}` }, code)),
          row.obsoleted ? h("span", { class: "badge text-bg-secondary", title: "The RFC is obsoleted" }, "obsoleted") : null,
        ),
      ),
      h("td", {}, artifactActions(row)),
    );
  }

  function sortArtifactsBy(key: ArtifactSortKey): void {
    artifactSort = nextArtifactSort(artifactSort, key);
    sortedArtifacts = sortArtifacts(artifactList?.artifacts ?? [], artifactSort);
    artifactOffset = 0;
    syncHash();
    renderArtifacts();
  }

  function artifactHeader(column: { key: ArtifactSortKey; label: string }): HTMLElement {
    const active = artifactSort?.key === column.key ? artifactSort.direction : null;
    const arrow = active === "asc" ? " \u25B2" : active === "desc" ? " \u25BC" : "";
    return h(
      "th",
      { "aria-sort": active === "asc" ? "ascending" : active === "desc" ? "descending" : undefined },
      h(
        "button",
        {
          class: `sort-button${active ? " active" : ""}`,
          type: "button",
          title: column.key === "findings" ? "Sort by severity" : `Sort by ${column.label.toLowerCase()}`,
          onclick: () => sortArtifactsBy(column.key),
        },
        column.label + arrow,
      ),
    );
  }

  function renderArtifacts(): void {
    const list = artifactList;
    if (!list) return;
    const total = list.artifacts.length;
    const page = sortedArtifacts.slice(artifactOffset, artifactOffset + PAGE_SIZE);
    const broken = list.broken.length;
    artifactsSummary.textContent = `${total} example${total === 1 ? "" : "s"}${broken ? `, ${broken} broken block${broken === 1 ? "" : "s"}` : ""} · data ${list.build}`;
    const pager =
      total > PAGE_SIZE
        ? h(
            "div",
            { class: "d-flex align-items-center gap-2 my-2 small" },
            h("button", { class: "btn btn-sm btn-outline-secondary", type: "button", disabled: artifactOffset === 0, onclick: () => goTo(artifactOffset - PAGE_SIZE) }, "Previous"),
            h("button", { class: "btn btn-sm btn-outline-secondary", type: "button", disabled: artifactOffset + PAGE_SIZE >= total, onclick: () => goTo(artifactOffset + PAGE_SIZE) }, "Next"),
            h("span", { class: "text-secondary" }, `${artifactOffset + 1}–${Math.min(artifactOffset + PAGE_SIZE, total)} of ${total}`),
          )
        : null;
    const brokenList = broken
      ? h(
          "details",
          { class: "my-3" },
          h("summary", { class: "small" }, `${broken} broken block${broken === 1 ? "" : "s"}: not terminated, not base64 or not BER`),
          h(
            "ul",
            { class: "small mt-2" },
            ...list.broken.map((b) => h("li", {}, h("span", { class: "font-monospace" }, `${b.id} ${b.label}`), " · ", documentLink(b), h("div", { class: "text-secondary" }, b.message))),
          ),
        )
      : null;
    replace(
      artifactsTable,
      total === 0
        ? h("div", { class: "text-secondary p-2" }, documentFilter.value.trim() ? `No PEM examples in ${documentFilter.value.trim()} match.` : "No examples match.")
        : h(
            "div",
            { class: "table-responsive" },
            h(
              "table",
              { class: "table table-sm align-middle" },
              h("thead", {}, h("tr", {}, ...ARTIFACT_COLUMNS.map(artifactHeader), h("th", {}))),
              h("tbody", {}, ...page.map(artifactRow)),
            ),
          ),
      pager,
      brokenList,
    );
  }

  function goTo(offset: number): void {
    artifactOffset = Math.max(0, offset);
    renderArtifacts();
    artifactsPane.scrollTo({ top: 0 });
  }

  for (const input of [documentFilter, typeFilter]) input.addEventListener("input", artifactsLater);
  for (const input of [labelFilter, confidenceFilter, errorsFilter]) input.addEventListener("change", () => void runArtifacts());

  const artifactsPane = h(
    "div",
    { class: "workbench-pane workbench-artifacts p-3" },
    h(
      "div",
      { class: "d-flex flex-wrap align-items-center gap-2" },
      h("div", { class: "workbench-filter" }, documentFilter),
      labelFilter,
      h("div", { class: "workbench-filter" }, typeFilter),
      confidenceFilter,
      h("div", { class: "form-check mb-0" }, errorsFilter, h("label", { class: "form-check-label small", for: "workbench-errors", title: "Examples identified as a type whose decode has errors: erratum candidates" }, "Example has errors")),
    ),
    artifactsSummary,
    artifactsAlert,
    artifactsTable,
  );

  // Codegen ----------------------------------------------------------------------------------

  const backendSelect = h(
    "select",
    { class: "form-select form-select-sm w-auto", "aria-label": "Backend" },
    h("option", { value: DEFAULT_BACKEND }, "rasn (Rust)"),
  );
  let backendsLoaded = false;
  let codegenController: AbortController | null = null;
  let codegen: CodegenResponse | null = null;
  /** What the code shown was generated for, so that showing the tab again does not generate it again. */
  let codegenKey = "";
  let fileIndex = 0;
  let viewer: EditorHandle | null = null;
  /** The Rust language for the viewer, loaded with the first generated code: only the Codegen tab needs it. */
  let rustLanguage: Promise<Extension> | null = null;
  const loadRust = (): Promise<Extension> =>
    (rustLanguage ??= import("@codemirror/lang-rust").then(
      (module) => module.rust(),
      () => [],
    ));
  const codegenStatus = h("span", { class: "small text-secondary", role: "status", "aria-live": "polite" });
  const codegenAlert = h("div");
  const fileTabs = h("div", { class: "d-flex flex-wrap gap-1", role: "tablist", "aria-label": "Generated files" });
  const copyButton = h("button", { class: "btn btn-sm btn-outline-secondary", type: "button", disabled: true }, "Copy");
  const downloadButton = h("button", { class: "btn btn-sm btn-outline-secondary", type: "button", disabled: true }, "Download file");
  const zipButton = h("button", { class: "btn btn-sm btn-outline-primary", type: "button", disabled: true }, "Download all (.zip)");
  const editorHost = h("div", { class: "workbench-code" });
  const codegenFindings = h("div", { class: "workbench-codegen-findings small" });

  async function loadBackends(): Promise<void> {
    if (backendsLoaded) return;
    backendsLoaded = true;
    try {
      const { backends } = await api.backends();
      const chosen = backendSelect.value;
      replace(backendSelect, ...backends.map((b: BackendMeta) => h("option", { value: b.id, title: `${b.library} ${b.library_version}` }, b.display_name)));
      if (backends.some((b) => b.id === chosen)) backendSelect.value = chosen;
    } catch (error) {
      backendsLoaded = false;
      console.error("glint: the backends could not be listed:", error);
    }
  }

  function currentFile(): { path: string; contents: string } | null {
    return codegen?.files[fileIndex] ?? null;
  }

  let language: Extension | null = null;

  function showFile(index: number): void {
    fileIndex = index;
    const file = currentFile();
    viewer ??= createEditor(editorHost, { readOnly: true, language: language ?? [] });
    viewer.setText(file?.contents ?? "");
    viewer.view.scrollDOM.scrollTop = 0;
    for (const [i, button] of Array.from(fileTabs.children).entries()) {
      const active = i === index;
      button.classList.toggle("btn-secondary", active);
      button.classList.toggle("btn-outline-secondary", !active);
      button.setAttribute("aria-selected", String(active));
    }
    copyButton.disabled = downloadButton.disabled = file === null;
  }

  function renderCodegen(message?: string): void {
    replace(codegenAlert, message ? h("div", { class: "alert alert-secondary py-1 small mb-0" }, message) : null);
    if (!codegen) {
      replace(fileTabs);
      replace(codegenFindings);
      codegenStatus.textContent = "";
      zipButton.disabled = true;
      viewer?.setText("");
      copyButton.disabled = downloadButton.disabled = true;
      return;
    }
    const { backend, files, findings } = codegen;
    const lines = files.filter((f) => f.path !== "glint_rt.rs").reduce((n, f) => n + f.contents.split("\n").length, 0);
    const modules = files.filter((f) => f.path !== "glint_rt.rs" && f.path !== "mod.rs").length;
    codegenStatus.textContent = `${codegen.root} · ${modules} ${modules === 1 ? "module" : "modules"}, ${lines.toLocaleString()} lines · ${backend.library} ${backend.library_version}`;
    replace(
      fileTabs,
      ...files.map((file, i) =>
        h(
          "button",
          {
            type: "button",
            role: "tab",
            class: "btn btn-sm btn-outline-secondary font-monospace",
            title: file.path === "glint_rt.rs" ? "The runtime the generated code needs, vendored (MIT-0)" : file.path === "mod.rs" ? "Declares the modules" : `ASN.1 module ${file.path}`,
            onclick: () => showFile(i),
          },
          file.path,
        ),
      ),
    );
    replace(
      codegenFindings,
      findings.length === 0
        ? h("div", { class: "text-secondary px-3 py-2" }, "No findings: everything the types say is expressed in the generated code, constraints included.")
        : h(
            "ul",
            { class: "list-unstyled mb-0 px-3 py-2" },
            ...findings.map((f) =>
              h(
                "li",
                { class: "mb-1" },
                h("span", { class: `badge ${SEVERITY_BADGE[f.severity]} me-2`, title: `glint explain ${f.code}` }, f.code),
                h("code", { class: "me-2" }, f.at),
                f.message,
              ),
            ),
          ),
    );
    zipButton.disabled = false;
    showFile(Math.min(fileIndex, files.length - 1));
  }

  async function runCodegen(): Promise<void> {
    codegenController?.abort();
    if (!picked?.type) {
      codegen = null;
      codegenKey = "";
      renderCodegen("Pick a module and a type above to generate code for it.");
      return;
    }
    const module: ModuleRef = { source: picked.source, name: picked.module, ...(picked.oid ? { oid: picked.oid } : {}) };
    const schema = schemaOf(module);
    if (typeof schema === "string") {
      codegen = null;
      codegenKey = "";
      renderCodegen(schema);
      return;
    }
    const backend = backendSelect.value || DEFAULT_BACKEND;
    const key = JSON.stringify([picked, backend, schema.document?.content ?? null]);
    if (key === codegenKey && codegen) return;
    const run = new AbortController();
    codegenController = run;
    codegenStatus.textContent = "Generating…";
    replace(codegenAlert);
    try {
      const [answer, loaded] = await Promise.all([api.codegen({ ...schema, type: picked.type, backend }, run.signal), loadRust()]);
      language = loaded;
      if (run.signal.aborted) return;
      codegen = answer;
      codegenKey = key;
      fileIndex = 0;
      renderCodegen();
    } catch (error) {
      if (isAbort(error) || run.signal.aborted) return;
      codegen = null;
      codegenKey = "";
      renderCodegen();
      replace(codegenAlert, h("div", { class: "alert alert-danger py-1 small mb-0", role: "alert" }, describe(error)));
    }
  }

  copyButton.addEventListener("click", () => {
    const file = currentFile();
    if (!file) return;
    void navigator.clipboard.writeText(file.contents).then(
      () => {
        copyButton.textContent = "Copied";
        setTimeout(() => (copyButton.textContent = "Copy"), 1500);
      },
      () => (copyButton.textContent = "Copy failed"),
    );
  });
  downloadButton.addEventListener("click", () => {
    const file = currentFile();
    if (file) saveFile(file.path, file.contents, "text/plain");
  });
  zipButton.addEventListener("click", () => {
    if (!codegen || !picked) return;
    // a folder the files can be dropped into as one Rust module (`mod.rs` declares the others)
    const folder = picked.type.toLowerCase().replace(/[^a-z0-9]+/g, "_").replace(/^_|_$/g, "") || "generated";
    const archive = zip(codegen.files.map((f) => ({ path: `${folder}/${f.path}`, contents: f.contents })));
    saveFile(`${folder}-${codegen.backend.id}.zip`, archive, "application/zip");
  });
  backendSelect.addEventListener("change", () => {
    syncHash();
    void runCodegen();
  });

  const codegenPane = h(
    "div",
    { class: "workbench-pane workbench-codegen" },
    h(
      "div",
      { class: "d-flex flex-column gap-2 p-3 pb-2 flex-shrink-0" },
      h("div", { class: "d-flex flex-wrap align-items-center gap-2" }, backendSelect, copyButton, downloadButton, zipButton, codegenStatus),
      codegenAlert,
      fileTabs,
    ),
    editorHost,
    codegenFindings,
  );

  replace(root, nav, pickerBar, decodePane, artifactsPane, codegenPane);
  renderBlock();
  describePick();

  return {
    show(route) {
      tab = route.tab;
      decodePane.hidden = tab !== "decode";
      artifactsPane.hidden = tab !== "artifacts";
      codegenPane.hidden = tab !== "codegen";
      pickerBar.hidden = tab === "artifacts";
      noSchemaCheck.hidden = tab === "codegen";
      decodeLink.classList.toggle("active", tab === "decode");
      artifactsLink.classList.toggle("active", tab === "artifacts");
      codegenLink.classList.toggle("active", tab === "codegen");
      const backend = route.backend ?? DEFAULT_BACKEND;
      if (backendSelect.value !== backend) {
        if (!Array.from(backendSelect.options).some((o) => o.value === backend)) backendSelect.append(h("option", { value: backend }, backend));
        backendSelect.value = backend;
      }
      const next: Picked | null =
        route.source && route.module ? { source: route.source, module: route.module, ...(route.moduleOid ? { oid: route.moduleOid } : {}), type: route.type ?? "" } : null;
      const pickChanged = !samePick(next, picked);
      if (pickChanged) setPicker(next);
      let documentChanged = false;
      if ((route.document ?? "") !== documentFilter.value.trim()) {
        // A link to a document's examples shows all of them: the filters the hash does not hold start over.
        documentFilter.value = route.document ?? "";
        labelFilter.value = "";
        typeFilter.value = "";
        confidenceFilter.value = "";
        errorsFilter.checked = false;
        documentChanged = true;
      }
      const sort = parseArtifactSort(route.sort);
      if ((sort ? formatArtifactSort(sort) : "") !== (artifactSort ? formatArtifactSort(artifactSort) : "")) {
        artifactSort = sort;
        sortedArtifacts = sortArtifacts(artifactList?.artifacts ?? [], artifactSort);
        artifactOffset = 0;
        renderArtifacts();
      }
      if (route.artifact && route.artifact !== artifactId) void loadArtifact(route.artifact, route);
      else if (pickChanged) void runDecode();
      if (tab === "artifacts" && (!artifactsLoaded || documentChanged)) void runArtifacts();
      if (tab === "codegen") {
        void loadBackends();
        void runCodegen();
      }
      describePick();
      syncHash();
    },
  };
}
