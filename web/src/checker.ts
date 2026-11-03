import type { Action, EditorHandle, LintDiagnostic } from "./editor";
import { createEditor } from "./editor";
import {
  ApiError,
  type CheckOptions,
  type CheckResponse,
  type Diagnostic,
  type FormatName,
  type Label,
  api,
} from "./api";
import { append, debounce, h, replace } from "./dom";
import { OffsetMap } from "./offsets";
import { hashForTarget, moduleHash, oidHash, type Route } from "./router";

const AUTO_CHECK_MS = 500;
const AUTO_CHECK_MAX_BYTES = 200_000;
const MAX_UPLOAD_BYTES = 8_000_000;

const SEVERITY_BADGE: Record<Diagnostic["severity"], string> = {
  error: "text-bg-danger",
  warning: "text-bg-warning",
  info: "text-bg-info",
};

function formatFromName(name: string): FormatName | null {
  const lower = name.toLowerCase();
  if (lower.endsWith(".xml")) return "xml";
  if (lower.endsWith(".md") || lower.endsWith(".markdown")) return "markdown";
  if (lower.endsWith(".asn") || lower.endsWith(".asn1")) return "bare";
  if (lower.endsWith(".txt")) return "text";
  return null;
}

export interface CheckerView {
  show(route: Extract<Route, { view: "checker" }>): void;
}

export function createChecker(root: HTMLElement): CheckerView {
  let editor: EditorHandle;
  let controller: AbortController | null = null;
  let sequence = 0;
  let documentName = "document";
  let loadedTarget = "";

  const target = h("input", {
    class: "form-control",
    type: "text",
    placeholder: "draft-ietf-lamps-foo-03 or RFC 5280",
    "aria-label": "Draft name or RFC number",
  });
  const formatSelect = h(
    "select",
    { class: "form-select form-select-sm w-auto", "aria-label": "Document format" },
    h("option", { value: "" }, "Auto-detect"),
    h("option", { value: "xml" }, "RFCXML"),
    h("option", { value: "markdown" }, "Markdown"),
    h("option", { value: "text" }, "Plain text"),
    h("option", { value: "bare" }, "ASN.1 module"),
  );
  const snippets = h("input", { class: "form-check-input", type: "checkbox", id: "opt-snippets", checked: true });
  const auto = h("input", { class: "form-check-input", type: "checkbox", id: "opt-auto", checked: true });
  const status = h("span", { class: "small text-secondary", role: "status", "aria-live": "polite" });
  const alertBox = h("div", { class: "px-2" });
  const editorHost = h("div", { class: "editor-host" });
  const diagnosticsPane = h("div", { class: "list-group list-group-flush" });
  const modulesPane = h("div");
  const importsPane = h("div");
  const diagnosticsBadge = h("span", { class: "badge text-bg-secondary ms-1" }, "0");
  const leftPane = h("div", { class: "pane pane-left", "data-drop-hint": "Drop a file to check it" });
  const fileInput = h("input", { type: "file", hidden: true, accept: ".xml,.txt,.md,.markdown,.asn,.asn1,text/*" });

  function options(): CheckOptions {
    return { snippets: snippets.checked };
  }

  function selectedFormat(): FormatName | null {
    return (formatSelect.value || null) as FormatName | null;
  }

  function showError(message: string | null): void {
    replace(
      alertBox,
      message ? h("div", { class: "alert alert-danger py-1 my-2 small", role: "alert" }, message) : null,
    );
  }

  function describe(error: unknown): string {
    if (error instanceof ApiError) {
      switch (error.code) {
        case "payload_too_large":
          return "The document is larger than the server accepts.";
        case "check_timeout":
          return "The check took too long and was stopped.";
        case "not_found":
          return "That draft or RFC was not found.";
        default:
          return error.status === 429 ? "Too many requests; wait a moment and try again." : error.message;
      }
    }
    return error instanceof Error ? error.message : String(error);
  }

  async function runCheck(): Promise<void> {
    controller?.abort();
    const mine = ++sequence;
    const run = new AbortController();
    controller = run;
    const text = editor.text();
    status.textContent = "Checking…";
    try {
      const response = await api.check(text, documentName, selectedFormat(), options(), run.signal);
      if (mine !== sequence) return;
      apply(response, text);
    } catch (error) {
      if (mine !== sequence || (error instanceof DOMException && error.name === "AbortError")) return;
      status.textContent = "";
      showError(describe(error));
    }
  }

  async function loadTarget(route: Extract<Route, { view: "checker" }>): Promise<void> {
    controller?.abort();
    const mine = ++sequence;
    const run = new AbortController();
    controller = run;
    const label = route.draft ?? `rfc${route.rfc}`;
    target.value = label;
    status.textContent = `Fetching ${label}…`;
    showError(null);
    try {
      const request = route.rfc !== undefined ? { rfc: route.rfc } : { draft: route.draft ?? "" };
      const response = await api.checkDraft(request, options(), run.signal);
      if (mine !== sequence) return;
      const content = response.document.content ?? "";
      documentName = response.document.name;
      formatSelect.value = "";
      editor.setHighlighting(response.document.format === "bare");
      editor.setText(content);
      if (editor.text() === content) {
        apply(response, content);
      } else {
        formatSelect.value = response.document.format;
        await runCheck();
      }
    } catch (error) {
      if (mine !== sequence || (error instanceof DOMException && error.name === "AbortError")) return;
      status.textContent = "";
      showError(describe(error));
    }
  }

  function toMarker(diagnostic: Diagnostic, response: CheckResponse, offsets: OffsetMap): LintDiagnostic | null {
    const documentId = response.files.find((f) => f.kind === "document")?.id;
    if (diagnostic.primary.file !== documentId) return null;
    const length = offsets.text.length;
    const from = offsets.toIndex(diagnostic.primary.span.start);
    let to = offsets.toIndex(diagnostic.primary.span.end);
    if (to <= from) to = Math.min(from + 1, length);
    const actions: Action[] = [];
    for (const secondary of diagnostic.secondary) {
      const where = describeLabel(secondary, response);
      const file = response.files.find((f) => f.id === secondary.file);
      if (file?.kind === "document") {
        const a = offsets.toIndex(secondary.span.start);
        const b = offsets.toIndex(secondary.span.end);
        actions.push({ name: `Go to ${where}`, apply: () => editor.reveal(a, b) });
      } else if (file?.module) {
        const module = file.module;
        actions.push({
          name: `Open ${module.source}#${module.name}`,
          apply: () => {
            window.location.hash = moduleHash(module.source, module.name, secondary.span);
          },
        });
      }
    }
    return {
      from: Math.min(from, length),
      to: Math.min(to, length),
      severity: diagnostic.severity,
      source: diagnostic.code,
      message: [diagnostic.message, ...diagnostic.notes, diagnostic.help ? `Help: ${diagnostic.help}` : ""]
        .filter(Boolean)
        .join("\n"),
      actions,
    };
  }

  function describeLabel(label: Label, response: CheckResponse): string {
    const file = response.files.find((f) => f.id === label.file);
    const place = `${label.line_col[0]}:${label.line_col[1]}`;
    return file && file.kind === "corpus" ? `${file.name}:${place}` : `line ${place}`;
  }

  function apply(response: CheckResponse, text: string): void {
    const offsets = new OffsetMap(text);
    showError(null);
    if (!formatSelect.value) editor.setHighlighting(response.document.format === "bare");
    const markers = response.diagnostics
      .map((d) => toMarker(d, response, offsets))
      .filter((m): m is LintDiagnostic => m !== null);
    editor.setMarkers(markers);
    renderDiagnostics(response, offsets);
    renderModules(response, offsets);
    renderImports(response);
    const errors = response.diagnostics.filter((d) => d.severity === "error").length;
    const warnings = response.diagnostics.filter((d) => d.severity === "warning").length;
    status.textContent = `${errors} error${errors === 1 ? "" : "s"}, ${warnings} warning${warnings === 1 ? "" : "s"} · corpus ${response.corpus_version}`;
  }

  function labelLink(label: Label, response: CheckResponse, offsets: OffsetMap): HTMLElement {
    const file = response.files.find((f) => f.id === label.file);
    const where = describeLabel(label, response);
    const text = label.text ? `${label.text} — ${where}` : where;
    if (file?.module) {
      return h("a", { href: moduleHash(file.module.source, file.module.name, label.span) }, text);
    }
    return h(
      "a",
      {
        href: "#",
        onclick: (event) => {
          event.preventDefault();
          editor.reveal(offsets.toIndex(label.span.start), offsets.toIndex(label.span.end));
        },
      },
      text,
    );
  }

  function renderDiagnostics(response: CheckResponse, offsets: OffsetMap): void {
    diagnosticsBadge.textContent = String(response.diagnostics.length);
    diagnosticsBadge.className = `badge ms-1 ${response.diagnostics.some((d) => d.severity === "error") ? "text-bg-danger" : "text-bg-secondary"}`;
    if (response.diagnostics.length === 0) {
      replace(diagnosticsPane, h("div", { class: "p-3 text-success" }, "No findings."));
      return;
    }
    replace(
      diagnosticsPane,
      ...response.diagnostics.map((d) => {
        const inDocument = response.files.find((f) => f.id === d.primary.file)?.kind === "document";
        const open = () => {
          if (inDocument) editor.reveal(offsets.toIndex(d.primary.span.start), offsets.toIndex(d.primary.span.end));
        };
        return h(
          "div",
          { class: `list-group-item diag diag-${d.severity}` },
          h(
            "div",
            { class: "d-flex gap-2 align-items-baseline" },
            h("span", { class: `badge ${SEVERITY_BADGE[d.severity]}` }, d.code),
            h(
              "a",
              {
                class: "diag-message flex-grow-1",
                href: "#",
                onclick: (event) => {
                  event.preventDefault();
                  open();
                },
              },
              d.message,
            ),
            h("span", { class: "small text-secondary text-nowrap" }, describeLabel(d.primary, response)),
          ),
          d.primary.section ? h("div", { class: "small text-secondary" }, `Section ${d.primary.section}`) : null,
          ...d.secondary.map((s) => h("div", { class: "small ps-3" }, "↳ ", labelLink(s, response, offsets))),
          ...d.notes.map((n) => h("div", { class: "small text-secondary ps-3" }, n)),
          d.help ? h("div", { class: "small fst-italic ps-3" }, d.help) : null,
        );
      }),
    );
  }

  function moduleStatus(response: CheckResponse, start: number, end: number): HTMLElement {
    const documentId = response.files.find((f) => f.kind === "document")?.id;
    const inside = response.diagnostics.filter(
      (d) => d.primary.file === documentId && d.primary.span.start >= start && d.primary.span.start < end,
    );
    const errors = inside.filter((d) => d.severity === "error").length;
    const warnings = inside.filter((d) => d.severity === "warning").length;
    if (errors > 0) return h("span", { class: "badge text-bg-danger" }, `${errors} error${errors === 1 ? "" : "s"}`);
    if (warnings > 0) return h("span", { class: "badge text-bg-warning" }, `${warnings} warning${warnings === 1 ? "" : "s"}`);
    return h("span", { class: "badge text-bg-success" }, "clean");
  }

  function renderModules(response: CheckResponse, offsets: OffsetMap): void {
    const jump = (start: number) => (event: Event) => {
      event.preventDefault();
      const index = offsets.toIndex(start);
      editor.reveal(index, index);
    };
    const moduleRows = response.modules.map((m) =>
      h(
        "tr",
        {},
        h("td", {}, h("a", { href: "#", onclick: jump(m.start) }, m.name)),
        h("td", { class: "font-monospace small" }, m.oid ? h("a", { href: oidHash(m.oid) }, m.oid) : "—"),
        h("td", {}, m.section ?? "—"),
        h("td", {}, moduleStatus(response, m.start, m.end)),
      ),
    );
    const fragments = response.blocks.filter((b) => b.kind === "fragment");
    replace(
      modulesPane,
      response.modules.length === 0
        ? h("div", { class: "p-3 text-secondary" }, "No ASN.1 modules were found in this document.")
        : h(
            "table",
            { class: "table table-sm align-middle mb-0" },
            h("thead", {}, h("tr", {}, h("th", {}, "Module"), h("th", {}, "OID"), h("th", {}, "Section"), h("th", {}, "Status"))),
            h("tbody", {}, ...moduleRows),
          ),
      fragments.length === 0
        ? null
        : h(
            "div",
            { class: "p-2 border-top" },
            h("div", { class: "small fw-semibold mb-1" }, `${fragments.length} snippet${fragments.length === 1 ? "" : "s"} in the text`),
            h(
              "ul",
              { class: "list-unstyled small mb-0" },
              ...fragments.map((f) =>
                h(
                  "li",
                  {},
                  h("a", { href: "#", onclick: jump(f.start) }, `line ${f.line}`),
                  f.section ? ` · section ${f.section}` : "",
                  f.title ? ` · ${f.title}` : "",
                ),
              ),
            ),
          ),
    );
  }

  function renderImports(response: CheckResponse): void {
    const withImports = response.modules.filter((m) => m.imports.length > 0);
    if (withImports.length === 0) {
      replace(importsPane, h("div", { class: "p-3 text-secondary" }, "No imports."));
      return;
    }
    replace(
      importsPane,
      ...withImports.map((m) =>
        h(
          "div",
          { class: "mb-3" },
          h("div", { class: "px-2 pt-2 fw-semibold small" }, m.name),
          h(
            "table",
            { class: "table table-sm align-middle mb-0" },
            h("thead", {}, h("tr", {}, h("th", {}, "Imported from"), h("th", {}, "Resolved to"), h("th", {}, "Symbols"))),
            h(
              "tbody",
              {},
              ...m.imports.map((i) =>
                h(
                  "tr",
                  {},
                  h("td", {}, i.module),
                  h(
                    "td",
                    {},
                    i.target
                      ? h("a", { href: moduleHash(i.target.source, i.target.name) }, `${i.target.source}#${i.target.name}`)
                      : i.resolved_to
                        ? `${i.resolved_to} (this document)`
                        : h("span", { class: "text-warning-emphasis" }, "not resolved"),
                  ),
                  h("td", { class: "small font-monospace" }, i.symbols.join(", ")),
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }

  function tabButton(id: string, label: string, active: boolean, extra?: Node): HTMLElement {
    return h(
      "li",
      { class: "nav-item", role: "presentation" },
      h(
        "button",
        {
          class: `nav-link${active ? " active" : ""}`,
          id: `${id}-tab`,
          "data-bs-toggle": "tab",
          "data-bs-target": `#${id}`,
          type: "button",
          role: "tab",
          "aria-controls": id,
          "aria-selected": String(active),
        },
        label,
        extra,
      ),
    );
  }

  function tabPane(id: string, active: boolean, ...children: Node[]): HTMLElement {
    return h(
      "div",
      { class: `tab-pane fade${active ? " show active" : ""}`, id, role: "tabpanel", "aria-labelledby": `${id}-tab` },
      ...children,
    );
  }

  const scheduleAuto = debounce(() => {
    if (auto.checked && editor.text().length < AUTO_CHECK_MAX_BYTES) void runCheck();
  }, AUTO_CHECK_MS);

  async function loadFile(file: File): Promise<void> {
    if (file.size > MAX_UPLOAD_BYTES) {
      showError(`${file.name} is larger than ${MAX_UPLOAD_BYTES / 1_000_000} MB.`);
      return;
    }
    let text: string;
    try {
      text = await file.text();
    } catch {
      showError(`Could not read ${file.name}.`);
      return;
    }
    if (text.includes("\u0000")) {
      showError(`${file.name} does not look like a text file.`);
      return;
    }
    showError(null);
    documentName = file.name;
    formatSelect.value = formatFromName(file.name) ?? "";
    target.value = "";
    loadedTarget = "";
    editor.setHighlighting(formatSelect.value === "bare");
    editor.setText(text);
    history.replaceState(null, "", window.location.pathname);
    await runCheck();
  }

  fileInput.addEventListener("change", async () => {
    const file = fileInput.files?.[0];
    if (!file) return;
    await loadFile(file);
    fileInput.value = "";
  });

  // Drag and drop onto the left pane. `dragDepth` balances the enter/leave events that
  // fire for every child element the pointer crosses.
  let dragDepth = 0;
  const carriesFiles = (event: DragEvent): boolean => event.dataTransfer?.types.includes("Files") ?? false;
  function setDragging(on: boolean): void {
    leftPane.classList.toggle("dragging", on);
  }
  leftPane.addEventListener("dragenter", (event) => {
    if (!carriesFiles(event)) return;
    event.preventDefault();
    dragDepth++;
    setDragging(true);
  });
  leftPane.addEventListener("dragover", (event) => {
    if (!carriesFiles(event)) return;
    event.preventDefault();
    if (event.dataTransfer) event.dataTransfer.dropEffect = "copy";
  });
  leftPane.addEventListener("dragleave", (event) => {
    if (!carriesFiles(event)) return;
    dragDepth = Math.max(0, dragDepth - 1);
    if (dragDepth === 0) setDragging(false);
  });
  // Capture phase and stopPropagation: CodeMirror would otherwise also insert the dropped
  // file's text at the drop position.
  leftPane.addEventListener(
    "drop",
    (event) => {
    if (!carriesFiles(event)) return;
    event.preventDefault();
    event.stopPropagation();
    dragDepth = 0;
    setDragging(false);
    const files = event.dataTransfer?.files;
    const file = files?.[0];
    if (!file) return;
    if (files.length > 1) showError(`Only one file can be checked at a time; using ${file.name}.`);
    void loadFile(file);
    },
    true,
  );

  function submitTarget(): void {
    const hash = hashForTarget(target.value);
    if (!hash) {
      showError("Enter a draft name such as draft-ietf-lamps-foo-03, or an RFC number.");
      return;
    }
    if (window.location.hash === hash) {
      loadedTarget = "";
      void loadTarget(parseTarget(hash));
    } else {
      window.location.hash = hash;
    }
  }

  function parseTarget(hash: string): Extract<Route, { view: "checker" }> {
    const rfc = /^#rfc(\d+)$/.exec(hash);
    if (rfc?.[1]) return { view: "checker", rfc: Number(rfc[1]) };
    return { view: "checker", draft: hash.slice(1) };
  }

  root.className = "view view-checker";
  append(leftPane, [
    h(
      "form",
      {
        class: "toolbar p-2 border-bottom",
        onsubmit: (event) => {
          event.preventDefault();
          submitTarget();
        },
      },
      h("div", { class: "input-group input-group-sm mb-2" }, target, h("button", { class: "btn btn-outline-primary", type: "submit" }, "Fetch")),
      h(
        "div",
        { class: "d-flex flex-wrap gap-2 align-items-center" },
        h("button", { class: "btn btn-sm btn-primary", type: "button", onclick: () => void runCheck() }, "Check"),
        h("button", { class: "btn btn-sm btn-outline-secondary", type: "button", onclick: () => fileInput.click() }, "Upload…"),
        fileInput,
        formatSelect,
        h("div", { class: "form-check mb-0" }, snippets, h("label", { class: "form-check-label small", for: "opt-snippets" }, "Snippets")),
        h("div", { class: "form-check mb-0" }, auto, h("label", { class: "form-check-label small", for: "opt-auto" }, "Auto-check")),
        status,
      ),
    ),
    alertBox,
    editorHost,
  ]);
  replace(
    root,
    h(
      "div",
      { class: "checker" },
      leftPane,
      h(
        "div",
        { class: "pane pane-right" },
        h(
          "ul",
          { class: "nav nav-tabs px-2 pt-2", role: "tablist" },
          tabButton("pane-diagnostics", "Diagnostics", true, diagnosticsBadge),
          tabButton("pane-modules", "Modules", false),
          tabButton("pane-imports", "Imports", false),
        ),
        h(
          "div",
          { class: "tab-content pane-scroll" },
          tabPane("pane-diagnostics", true, diagnosticsPane),
          tabPane("pane-modules", false, modulesPane),
          tabPane("pane-imports", false, importsPane),
        ),
      ),
    ),
  );
  root.hidden = false;
  editor = createEditor(editorHost, { highlighting: false, onChange: scheduleAuto });
  root.hidden = true;
  replace(diagnosticsPane, h("div", { class: "p-3 text-secondary" }, "Paste ASN.1 or a draft on the left, upload a file, or fetch a draft or RFC."));

  return {
    show(route) {
      const wanted = route.rfc !== undefined ? `rfc${route.rfc}` : (route.draft ?? "");
      if (wanted && wanted !== loadedTarget) {
        loadedTarget = wanted;
        void loadTarget(route);
      }
    },
  };
}
