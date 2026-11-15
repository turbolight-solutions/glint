import type { EditorState, StateEffect, TransactionSpec } from "@codemirror/state";
import type { Action, EditorHandle, LintDiagnostic } from "./editor";
import { createEditor, highlightingSpec, markersSpec, replaceTextSpec } from "./editor";
import {
  ApiError,
  type AppliedErratum,
  type CheckOptions,
  type CheckResponse,
  type Diagnostic,
  type ErrataFailure,
  type FormatName,
  type HunkLines,
  type Label,
  type MarkupFindings,
  type ModuleInfo,
  api,
  moduleLabel,
} from "./api";
import { append, debounce, h, replace } from "./dom";
import { OffsetMap } from "./offsets";
import { draftHash, hashForTarget, moduleHash, oidHash, type Route, rfcHash } from "./router";
import { createTabStrip, nextActive } from "./tabs";

const AUTO_CHECK_MS = 500;
const AUTO_CHECK_MAX_BYTES = 1_000_000;
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

/** The errata the corpus applied to a document opened by name. */
interface TabErrata {
  applied: AppliedErratum[];
  failure: ErrataFailure | null;
  /**
   * The editor text the hunk lines refer to: jumps work while the editor still holds it. Null when the editor's lines
   * are not the corpus text's (it read a lone `\r` as a line break), so the line numbers cannot be trusted.
   */
  text: string | null;
}

export interface CheckerView {
  show(route: Extract<Route, { view: "checker" }>): void;
}

/** One open document. The editor view shows exactly one tab's state at a time. */
interface Tab {
  id: number;
  /** Name sent to the checker; null for pasted text. */
  name: string | null;
  /** Draft or RFC this tab was opened from, as it appears in the URL hash. */
  target: string;
  /** Value of the format select; empty means auto-detect. */
  format: string;
  /** Editor state while the tab is in the background. The view owns it while the tab is shown. */
  state: EditorState;
  scroll: StateEffect<unknown> | null;
  status: string;
  error: string | null;
  result: { response: CheckResponse; text: string; offsets: OffsetMap } | null;
  /** Errata of the corpus document the tab was opened from; null for any other text. */
  errata: TabErrata | null;
  /**
   * For an RFC or draft opened by name that the corpus has both as its text and as its RFCXML (an RFC's corrected by its
   * errata): the published XML's URL.
   */
  xmlSource: string | null;
  /**
   * For such a document: the route of its other view (the RFCXML from the text, the text from the RFCXML). A check of the
   * text view's edited text reads it with the text heuristics, not with the blocks the RFCXML marks.
   */
  otherView: CheckerRoute | null;
  fetching: boolean;
  /** Module to put the cursor at once the document and its check are on screen. */
  pendingModule: { name: string; oid?: string } | null;
  /** Modules whose panel in the Modules tab is expanded; they stay open across re-checks. */
  openModules: Set<string>;
  controller: AbortController | null;
  /** Bumped by every request and by closing, so a late answer can tell it was superseded. */
  sequence: number;
  scheduleAuto: () => void;
}

type CheckerRoute = Extract<Route, { view: "checker" }>;

/** The hash of a draft or RFC route: its text or its RFCXML. */
function viewHash(route: CheckerRoute): string {
  return route.rfc !== undefined ? rfcHash(route.rfc, route.xml) : draftHash(route.draft ?? "", route.xml);
}

function titleOf(tab: Tab): string {
  return tab.name ?? (tab.target || "Untitled");
}

/** How many findings about the RFCXML markup a text view's check counted. */
function markupCount(markup: MarkupFindings | undefined): number {
  return markup ? markup.errors + markup.warnings + markup.infos : 0;
}

export function createChecker(root: HTMLElement): CheckerView {
  let editor: EditorHandle;
  let tabs: Tab[] = [];
  let active: Tab;
  let nextId = 1;
  // Set while code (not the user) edits the shown document, so it does not schedule a check.
  let programmatic = false;
  // A hash the Open button set itself; its `hashchange` has already been acted on.
  let handledHash = "";

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
  // Links to the other view of an RFC the corpus has as text and as RFCXML, and to the published XML.
  const viewLinks = h("span", { class: "small text-nowrap view-links" });
  const alertBox = h("div", { class: "px-2" });
  const editorHost = h("div", { class: "editor-host" });
  const diagnosticsPane = h("div", { class: "list-group list-group-flush" });
  const modulesPane = h("div");
  // Read-only views inside the open module panels; they are rebuilt whenever the pane is.
  let moduleViews: EditorHandle[] = [];
  const importsPane = h("div");
  const errataPane = h("div");
  const diagnosticsBadge = h("span", { class: "badge text-bg-secondary ms-1" }, "0");
  const errataBadge = h("span", { class: "badge text-bg-secondary ms-1 d-none" }, "0");
  const leftPane = h("div", { class: "pane pane-left", "data-drop-hint": "Drop files to check them" });
  const fileInput = h("input", {
    type: "file",
    hidden: true,
    multiple: true,
    accept: ".xml,.txt,.md,.markdown,.asn,.asn1,text/*",
  });
  const strip = createTabStrip({
    select(id) {
      const tab = tabs.find((t) => t.id === id);
      if (tab) activate(tab);
    },
    close: (id) => closeTabs(new Set([id])),
    closeOthers: (id) => closeTabs(new Set(tabs.filter((t) => t.id !== id).map((t) => t.id))),
    closeAll: () => closeTabs(new Set(tabs.map((t) => t.id))),
    add() {
      const tab = createTab(editor.newState());
      tabs.push(tab);
      activate(tab);
      editor.view.focus();
    },
  });

  function options(): CheckOptions {
    return { snippets: snippets.checked };
  }

  function selectedFormat(tab: Tab): FormatName | null {
    return (tab.format || null) as FormatName | null;
  }

  function showError(message: string | null): void {
    replace(
      alertBox,
      message ? h("div", { class: "alert alert-danger py-1 my-2 small", role: "alert" }, message) : null,
    );
  }

  function describe(error: unknown): string {
    if (error instanceof ApiError && error.code === "documents_loading") {
      return "The document texts are still loading; try again in a moment.";
    }
    return error instanceof Error ? error.message : String(error);
  }

  // Tabs ---------------------------------------------------------------------------------

  const isActive = (tab: Tab): boolean => tab === active;
  const stateOf = (tab: Tab): EditorState => (isActive(tab) ? editor.view.state : tab.state);
  const textOf = (tab: Tab): string => stateOf(tab).doc.toString();
  /** A tab that can take a new document: nothing typed, and no fetch on its way in. */
  const isBlank = (tab: Tab): boolean => !tab.fetching && textOf(tab).trim() === "";

  /** Edits a tab's document whether or not it is the one on screen. */
  function edit(tab: Tab, spec: (state: EditorState) => TransactionSpec): void {
    if (isActive(tab)) {
      programmatic = true;
      try {
        editor.view.dispatch(spec(editor.view.state));
      } finally {
        programmatic = false;
      }
    } else {
      tab.state = tab.state.update(spec(tab.state)).state;
    }
  }

  function createTab(state: EditorState): Tab {
    const tab: Tab = {
      id: nextId++,
      name: null,
      target: "",
      format: "",
      state,
      scroll: null,
      status: "",
      error: null,
      result: null,
      errata: null,
      xmlSource: null,
      otherView: null,
      fetching: false,
      pendingModule: null,
      openModules: new Set(),
      controller: null,
      sequence: 0,
      scheduleAuto: debounce(() => {
        if (!auto.checked || !tabs.includes(tab)) return;
        if (textOf(tab).length >= AUTO_CHECK_MAX_BYTES) {
          setStatus(tab, "Document too large to auto-check; press Check to update the results.");
          return;
        }
        void runCheck(tab);
      }, AUTO_CHECK_MS),
    };
    return tab;
  }

  function renderTabs(): void {
    strip.render(
      tabs.map((t) => ({ id: t.id, title: titleOf(t), busy: t.fetching })),
      active.id,
    );
  }

  /** Redraws everything that belongs to the active tab, except the URL. */
  function refresh(): void {
    target.value = active.target;
    formatSelect.value = active.format;
    status.textContent = active.status;
    renderViewLinks(active);
    showError(active.error);
    renderResult(active);
    renderTabs();
  }

  /** Keeps the URL hash naming what the active tab holds, so a reload or a copy of the link matches. */
  function syncUrl(): void {
    const hash = active.target ? `#${active.target}` : "";
    if (window.location.hash !== hash) history.replaceState(null, "", window.location.pathname + hash);
  }

  function switchTo(tab: Tab): void {
    if (tab !== active) {
      active.state = editor.view.state;
      active.scroll = editor.scrollSnapshot();
      active = tab;
      editor.swap(tab.state, tab.scroll);
    }
    refresh();
  }

  function activate(tab: Tab): void {
    switchTo(tab);
    syncUrl();
    revealPendingModule(tab);
  }

  /** Puts the cursor at the start of the module a link asked for, once the check has found it. */
  function revealPendingModule(tab: Tab): void {
    const wanted = tab.pendingModule;
    if (!wanted || !tab.result || !isActive(tab)) return;
    tab.pendingModule = null;
    const named = tab.result.response.modules.filter((m) => m.name === wanted.name);
    const found = (wanted.oid ? named.find((m) => m.oid === wanted.oid) : undefined) ?? named[0];
    if (!found) return;
    const index = tab.result.offsets.toIndex(found.start);
    editor.reveal(index, index);
  }

  /** Brings a tab's own title, spinner and format choice up to date after it changed in the background. */
  function syncChrome(tab: Tab): void {
    if (isActive(tab)) formatSelect.value = tab.format;
    renderTabs();
  }

  function closeTabs(closing: ReadonlySet<number>): void {
    const keep = nextActive(
      tabs.map((t) => t.id),
      closing,
      active.id,
    );
    for (const tab of tabs) {
      if (!closing.has(tab.id)) continue;
      tab.controller?.abort();
      tab.sequence++;
    }
    const activeClosed = closing.has(active.id);
    tabs = tabs.filter((t) => !closing.has(t.id));
    if (activeClosed) {
      let next = tabs.find((t) => t.id === keep);
      if (!next) {
        // Closing the last tab leaves a fresh empty one: the editor always has something to show.
        next = createTab(editor.newState());
        tabs.push(next);
      }
      active = next;
      editor.swap(next.state, next.scroll);
    }
    refresh();
    syncUrl();
  }

  /**
   * The tab an incoming document goes into: the current one if it is empty, otherwise a new
   * one. Either way it is the active tab afterwards.
   */
  function claimTab(): Tab {
    if (isBlank(active)) return active;
    const tab = createTab(editor.newState());
    tabs.push(tab);
    switchTo(tab);
    return tab;
  }

  function setStatus(tab: Tab, text: string): void {
    tab.status = text;
    if (isActive(tab)) status.textContent = text;
  }

  function setError(tab: Tab, message: string | null): void {
    tab.error = message;
    if (isActive(tab)) showError(message);
  }

  /** Starts a request for a tab, cancelling whatever it had in flight. */
  function startRun(tab: Tab): { mine: number; signal: AbortSignal } {
    tab.controller?.abort();
    const run = new AbortController();
    tab.controller = run;
    if (tab.fetching) {
      tab.fetching = false;
      renderTabs();
    }
    return { mine: ++tab.sequence, signal: run.signal };
  }

  async function runCheck(tab: Tab): Promise<void> {
    const { mine, signal } = startRun(tab);
    const text = textOf(tab);
    setStatus(tab, "Checking…");
    try {
      const response = await api.check(text, tab.name ?? "document", selectedFormat(tab), options(), signal);
      if (mine !== tab.sequence) return;
      apply(tab, response, text);
    } catch (error) {
      if (mine !== tab.sequence || (error instanceof DOMException && error.name === "AbortError")) return;
      setStatus(tab, "");
      setError(tab, describe(error));
    }
  }

  /** What a route opens, as it appears in the URL hash and names the tab that holds it. */
  function targetOf(route: CheckerRoute): string {
    if (route.rfc !== undefined || route.draft) return viewHash(route).slice(1);
    // Each module of another body is a file of its own, so it names the tab along with its document.
    return route.document && route.module ? moduleHash(route.document, route.module, route.moduleOid).slice(1) : "";
  }

  async function loadTarget(route: CheckerRoute): Promise<void> {
    const tab = claimTab();
    const { mine, signal } = startRun(tab);
    tab.pendingModule = route.module ? { name: route.module, oid: route.moduleOid } : null;
    const label = targetOf(route);
    tab.target = label;
    tab.fetching = true;
    setStatus(tab, `Opening ${label}…`);
    setError(tab, null);
    target.value = label;
    renderTabs();
    syncUrl();
    try {
      if (route.document && route.module) {
        await loadModule(tab, mine, route.document, route.module, route.moduleOid);
        return;
      }
      const format = route.xml ? { format: "xml" as const } : {};
      const request = route.rfc !== undefined ? { rfc: route.rfc, ...format } : { draft: route.draft ?? "", ...format };
      const response = await api.checkDocument(request, options(), signal);
      if (mine !== tab.sequence) return;
      const content = response.document.content ?? "";
      tab.fetching = false;
      tab.name = response.document.name;
      tab.format = "";
      edit(tab, () => highlightingSpec(response.document.format === "bare"));
      edit(tab, (state) => replaceTextSpec(state, content));
      const sameLines = stateOf(tab).doc.lines === content.split("\n").length;
      tab.errata = {
        applied: response.document.errata ?? [],
        failure: response.document.errata_failure ?? null,
        text: sameLines ? textOf(tab) : null,
      };
      tab.xmlSource = response.document.xml_source ?? null;
      const other = response.document.other_format;
      const document = route.rfc !== undefined ? { rfc: route.rfc } : { draft: route.draft ?? "" };
      tab.otherView = other ? { view: "checker", ...document, ...(other === "xml" ? { xml: true } : {}) } : null;
      if (isActive(tab)) renderViewLinks(tab);
      if (textOf(tab) === content) {
        syncChrome(tab);
        apply(tab, response, content);
      } else {
        // The editor normalised the text (line endings), so the offsets in this response are off.
        tab.format = response.document.format;
        syncChrome(tab);
        await runCheck(tab);
      }
    } catch (error) {
      if (mine !== tab.sequence || (error instanceof DOMException && error.name === "AbortError")) return;
      tab.fetching = false;
      setStatus(tab, "");
      setError(tab, describe(error));
      renderTabs();
    }
  }

  /** A module of another standards body is a file of its own: its text is fetched from the corpus and checked as it is. */
  async function loadModule(tab: Tab, mine: number, document: string, name: string, oid?: string): Promise<void> {
    const module = await api.module(document, name, oid);
    if (mine !== tab.sequence) return;
    tab.fetching = false;
    tab.name = `${document}#${name}`;
    tab.format = "bare";
    tab.errata = null;
    tab.xmlSource = null;
    tab.otherView = null;
    if (isActive(tab)) renderViewLinks(tab);
    edit(tab, () => highlightingSpec(true));
    edit(tab, (state) => replaceTextSpec(state, module.text));
    syncChrome(tab);
    await runCheck(tab);
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
            window.location.hash = moduleHash(module.source, module.name, module.oid);
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

  function apply(tab: Tab, response: CheckResponse, text: string): void {
    const offsets = new OffsetMap(text);
    if (!tab.format) edit(tab, () => highlightingSpec(response.document.format === "bare"));
    const markers = response.diagnostics
      .map((d) => toMarker(d, response, offsets))
      .filter((m): m is LintDiagnostic => m !== null);
    edit(tab, (state) => markersSpec(state, markers));
    tab.result = { response, text, offsets };
    // Findings about the RFCXML markup are shown on the XML view but counted here too, so the text view (the one
    // opened by default) does not hide them.
    const markup = response.document.markup_findings;
    const errors = response.diagnostics.filter((d) => d.severity === "error").length + (markup?.errors ?? 0);
    const warnings = response.diagnostics.filter((d) => d.severity === "warning").length + (markup?.warnings ?? 0);
    const markupTotal = markupCount(markup);
    const ofMarkup = markupTotal > 0 ? ` (${markupTotal} about the RFCXML markup)` : "";
    setError(tab, null);
    // A check of the text view's edited text cannot use the blocks the RFCXML source marks: it reads the text, and
    // says so.
    const heuristic =
      tab.otherView?.xml && !response.document.other_format
        ? " · ASN.1 found by reading the edited text, not from the RFCXML marking"
        : "";
    setStatus(
      tab,
      `${errors} error${errors === 1 ? "" : "s"}, ${warnings} warning${warnings === 1 ? "" : "s"}${ofMarkup} · Glint ${response.glint_version}, data ${response.data_build}${heuristic}`,
    );
    if (isActive(tab)) {
      renderResult(tab);
      revealPendingModule(tab);
    }
  }

  /** Fills the right-hand panes from a tab's last check, or with the empty-state hint. */
  function renderResult(tab: Tab): void {
    renderErrata(tab);
    if (!tab.result) {
      diagnosticsBadge.textContent = "0";
      diagnosticsBadge.className = "badge text-bg-secondary ms-1";
      replace(
        diagnosticsPane,
        h("div", { class: "p-3 text-secondary" }, "Paste ASN.1 or a draft on the left, upload a file, or fetch a draft or RFC."),
      );
      destroyModuleViews();
      replace(modulesPane);
      replace(importsPane);
      return;
    }
    const { response, offsets } = tab.result;
    renderDiagnostics(tab, response, offsets);
    renderModules(tab, response, offsets);
    renderImports(response, offsets);
  }

  function labelLink(label: Label, response: CheckResponse, offsets: OffsetMap): HTMLElement {
    const file = response.files.find((f) => f.id === label.file);
    const where = describeLabel(label, response);
    const text = label.text ? `${label.text} — ${where}` : where;
    if (file?.module) {
      return h("a", { href: moduleHash(file.module.source, file.module.name, file.module.oid) }, text);
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

  /** The line that stands for the findings about the RFCXML marking a text view keeps out of its list. */
  function markupLine(tab: Tab, count: number): HTMLElement {
    const label = `${count} finding${count === 1 ? "" : "s"} about the RFCXML markup`;
    const other = tab.otherView;
    return h(
      "div",
      { class: "list-group-item small markup-findings" },
      other?.xml ? viewLink(other, label, `Show them on the elements of the ${other.rfc !== undefined ? "corrected " : ""}RFCXML`) : label,
    );
  }

  function renderDiagnostics(tab: Tab, response: CheckResponse, offsets: OffsetMap): void {
    const markupFindingsOf = response.document.markup_findings;
    const markup = markupCount(markupFindingsOf);
    diagnosticsBadge.textContent = String(response.diagnostics.length + markup);
    const anyError = response.diagnostics.some((d) => d.severity === "error") || (markupFindingsOf?.errors ?? 0) > 0;
    diagnosticsBadge.className = `badge ms-1 ${anyError ? "text-bg-danger" : "text-bg-secondary"}`;
    const markupFindings = markup > 0 ? markupLine(tab, markup) : null;
    if (response.diagnostics.length === 0) {
      replace(diagnosticsPane, markupFindings, h("div", { class: "p-3 text-success" }, markupFindings ? "No other findings." : "No findings."));
      return;
    }
    replace(
      diagnosticsPane,
      markupFindings,
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

  function destroyModuleViews(): void {
    for (const view of moduleViews) view.view.destroy();
    moduleViews = [];
  }

  /** One collapsible panel per module; its read-only view is only built once the panel is open. */
  function modulePanel(tab: Tab, response: CheckResponse, offsets: OffsetMap, m: ModuleInfo, jump: (start: number) => (event: Event) => void): HTMLElement {
    const from = offsets.toIndex(m.start);
    const to = offsets.toIndex(m.end);
    const host = h("div", { class: "module-panel-editor" });
    let built = false;
    const build = () => {
      if (built) return;
      built = true;
      let firstLine = 1;
      for (let at = offsets.text.indexOf("\n"); at !== -1 && at < from; at = offsets.text.indexOf("\n", at + 1)) firstLine++;
      moduleViews.push(createEditor(host, { readOnly: true, doc: offsets.text.slice(from, to), firstLine }));
    };
    const panel = h(
      "details",
      { class: "module-panel border-bottom" },
      h(
        "summary",
        { class: "d-flex flex-wrap align-items-center gap-2 px-2 py-1" },
        h("a", { class: "fw-semibold", href: "#", title: "Show in the editor", onclick: jump(m.start) }, m.name),
        m.oid ? h("a", { class: "font-monospace small", href: oidHash(m.oid) }, m.oid) : null,
        m.section ? h("span", { class: "small text-secondary" }, `section ${m.section}`) : null,
        moduleStatus(response, m.start, m.end),
      ),
      host,
    );
    panel.addEventListener("toggle", () => {
      if (panel.open) {
        tab.openModules.add(m.name);
        build();
      } else {
        tab.openModules.delete(m.name);
      }
    });
    if (tab.openModules.has(m.name)) {
      panel.open = true;
      build();
    }
    return panel;
  }

  function renderModules(tab: Tab, response: CheckResponse, offsets: OffsetMap): void {
    const jump = (start: number) => (event: Event) => {
      event.preventDefault();
      const index = offsets.toIndex(start);
      editor.reveal(index, index);
    };
    destroyModuleViews();
    const fragments = response.blocks.filter((b) => b.kind === "fragment");
    replace(
      modulesPane,
      response.modules.length === 0
        ? h("div", { class: "p-3 text-secondary" }, "No ASN.1 modules were found in this document.")
        : h("div", {}, ...response.modules.map((m) => modulePanel(tab, response, offsets, m, jump))),
      fragments.length === 0
        ? null
        : h(
            "div",
            { class: "p-2" },
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

  function renderImports(response: CheckResponse, offsets: OffsetMap): void {
    const withImports = response.modules.filter((m) => m.imports.length > 0);
    // An import resolved to a module of the document itself names that module; it links to the module in the editor.
    // Two modules of one name (RFC 4998's ERS) point at the other one than the importer.
    const local = (importer: ModuleInfo, name: string): HTMLElement | string => {
      const named = response.modules.filter((x) => x.name === name);
      const found = named.find((x) => x.start !== importer.start) ?? named[0];
      const label = `${name} (this document)`;
      if (!found) return label;
      const open = (event: Event) => {
        event.preventDefault();
        const index = offsets.toIndex(found.start);
        editor.reveal(index, index);
      };
      return h("a", { href: "#", title: "Show in the editor", onclick: open }, label);
    };
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
                      ? h("a", { href: moduleHash(i.target.source, i.target.name, i.target.oid) }, moduleLabel(i.target))
                      : i.resolved_to
                        ? local(m, i.resolved_to)
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

  /** Whether the editor still holds the text a tab's errata lines refer to. */
  function errataJumpable(tab: Tab): boolean {
    return tab.errata?.text != null && textOf(tab) === tab.errata.text;
  }

  function linesLabel(lines: HunkLines): string {
    if (lines.end_line < lines.start_line) return `removed before line ${lines.start_line}`;
    if (lines.end_line === lines.start_line) return `line ${lines.start_line}`;
    return `lines ${lines.start_line}–${lines.end_line}`;
  }

  /** The diff of one erratum, its hunk headers replaced by the lines each hunk is at in the editor. */
  function errataDiff(tab: Tab, erratum: AppliedErratum): HTMLElement {
    const diff = h("div", { class: "errata-diff font-monospace small" });
    let hunk = -1;
    let jump: ((event: Event) => void) | null = null;
    for (const line of erratum.patch.split("\n")) {
      // The file headers come before the first hunk; a removed `-- comment` line also starts with `---`.
      if (line === "" || (hunk < 0 && !line.startsWith("@@"))) continue;
      if (line.startsWith("@@")) {
        hunk++;
        const lines = erratum.hunks[hunk];
        jump = lines && errataJumpable(tab) ? jumpToLines(tab, lines) : null;
        const label = lines ? linesLabel(lines) : line;
        append(diff, [
          jump
            ? h("a", { class: "errata-hunk d-block", href: "#", title: "Show in the editor", onclick: jump }, label)
            : h("div", { class: "errata-hunk text-secondary" }, label),
        ]);
        continue;
      }
      const kind = line.startsWith("+") ? "add" : line.startsWith("-") ? "del" : "ctx";
      const lineJump = kind === "del" ? null : jump;
      append(diff, [
        h("div", { class: `diff-line diff-${kind}${lineJump ? " diff-jump" : ""}`, onclick: lineJump ?? undefined }, line),
      ]);
    }
    return diff;
  }

  function jumpToLines(tab: Tab, lines: HunkLines): (event: Event) => void {
    return (event) => {
      event.preventDefault();
      if (!errataJumpable(tab)) {
        // The text changed since the pane was drawn: say so instead of jumping to the wrong lines.
        if (isActive(tab)) renderErrata(tab);
        return;
      }
      editor.revealLines(lines.start_line, lines.end_line);
    };
  }

  function renderErrata(tab: Tab): void {
    const errata = tab.errata;
    const count = errata?.applied.length ?? 0;
    errataBadge.textContent = String(count);
    errataBadge.className = `badge text-bg-secondary ms-1${count === 0 ? " d-none" : ""}`;
    const failure = errata?.failure
      ? h(
          "div",
          { class: "alert alert-warning m-2 small", role: "alert" },
          `The errata of this RFC could not be applied (erratum ${errata.failure.errata_id}: ${errata.failure.reason}). The text is the published one.`,
        )
      : null;
    if (!errata || count === 0) {
      replace(errataPane, failure, h("div", { class: "p-3 text-secondary" }, "No errata applied."));
      return;
    }
    replace(
      errataPane,
      failure,
      errataJumpable(tab)
        ? null
        : h("div", { class: "px-3 pt-2 small text-secondary" }, "The text was changed after it was opened, so the lines below may no longer match; open the document again to jump to them."),
      h(
        "div",
        { class: "list-group list-group-flush" },
        ...errata.applied.map((erratum) =>
          h(
            "div",
            { class: "list-group-item erratum" },
            h(
              "div",
              { class: "d-flex flex-wrap gap-2 align-items-baseline" },
              h(
                "a",
                { class: "fw-semibold", href: `https://www.rfc-editor.org/errata/eid${erratum.id}`, target: "_blank", rel: "noopener" },
                `Erratum ${erratum.id}`,
              ),
              h("span", { class: "small text-secondary" }, `applied, decided by ${erratum.by}`),
            ),
            erratum.note ? h("div", { class: "small mb-1" }, erratum.note) : null,
            errataDiff(tab, erratum),
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

  /** Opens an uploaded file in the current tab if that is empty, otherwise in a new one. */
  async function loadFile(file: File): Promise<void> {
    if (file.size > MAX_UPLOAD_BYTES) {
      setError(active, `${file.name} is larger than ${MAX_UPLOAD_BYTES / 1_000_000} MB.`);
      return;
    }
    let text: string;
    try {
      text = await file.text();
    } catch {
      setError(active, `Could not read ${file.name}.`);
      return;
    }
    if (text.includes("\u0000")) {
      setError(active, `${file.name} does not look like a text file.`);
      return;
    }
    const tab = claimTab();
    tab.name = file.name;
    tab.format = formatFromName(file.name) ?? "";
    tab.target = "";
    tab.result = null;
    tab.errata = null;
    tab.xmlSource = null;
    tab.otherView = null;
    setError(tab, null);
    edit(tab, () => highlightingSpec(tab.format === "bare"));
    edit(tab, (state) => replaceTextSpec(state, text));
    refresh();
    syncUrl();
    void runCheck(tab);
  }

  async function loadFiles(files: File[]): Promise<void> {
    for (const file of files) await loadFile(file);
  }

  fileInput.addEventListener("change", async () => {
    // Copy first: the FileList is live and empties when the input is reset.
    const files = [...(fileInput.files ?? [])];
    fileInput.value = "";
    await loadFiles(files);
  });

  formatSelect.addEventListener("change", () => {
    active.format = formatSelect.value;
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
    void loadFiles([...(event.dataTransfer?.files ?? [])]);
    },
    true,
  );

  function submitTarget(): void {
    const hash = hashForTarget(target.value);
    if (!hash) {
      setError(active, "Enter a draft name such as draft-ietf-lamps-foo-03, or an RFC number.");
      return;
    }
    // Fetch always fetches, even for a document that is already open in another tab. Setting the
    // hash keeps the history entry; `show` must not act on the `hashchange` it causes.
    if (window.location.hash !== hash) {
      handledHash = hash.slice(1);
      window.location.hash = hash;
    }
    void loadTarget(parseTarget(hash));
  }

  function parseTarget(hash: string): CheckerRoute {
    const rfc = /^#rfc(\d+)(\.xml)?$/.exec(hash);
    if (rfc?.[1]) return { view: "checker", rfc: Number(rfc[1]), ...(rfc[2] ? { xml: true } : {}) };
    const draft = /^#(.*?)(\.xml)?$/.exec(hash);
    return { view: "checker", draft: draft?.[1] ?? "", ...(draft?.[2] ? { xml: true } : {}) };
  }

  /** Shows the tab that holds a route, or opens it in a tab of its own. */
  function openRoute(route: CheckerRoute): void {
    const open = tabs.find((t) => t.target === targetOf(route));
    if (open) {
      open.pendingModule = route.module ? { name: route.module, oid: route.moduleOid } : null;
      activate(open);
    } else {
      void loadTarget(route);
    }
  }

  /** A link that opens the other view of an RFC or draft, or brings its tab forward. */
  function viewLink(route: CheckerRoute, label: string, title: string): HTMLElement {
    return h(
      "a",
      {
        href: viewHash(route),
        title,
        onclick: (event) => {
          event.preventDefault();
          openRoute(route);
        },
      },
      label,
    );
  }

  /** The links beside the status line: the other view of the active RFC or draft, and its published RFCXML. */
  function renderViewLinks(tab: Tab): void {
    const other = tab.otherView;
    const rfc = other?.rfc !== undefined;
    const title = other?.xml
      ? rfc
        ? "Open the corrected RFCXML of this RFC, with its findings about the markup"
        : "Open the RFCXML of this draft, with its findings about the markup"
      : rfc
        ? "Open the published text of this RFC, with its errata applied"
        : "Open the published text of this draft";
    replace(
      viewLinks,
      other ? viewLink(other, other.xml ? "RFCXML" : "Text", title) : null,
      other && tab.xmlSource ? " " : null,
      other && tab.xmlSource
        ? h(
            "a",
            {
              class: "text-secondary",
              href: tab.xmlSource,
              target: "_blank",
              rel: "noopener",
              title: rfc ? "The RFCXML as the RFC Editor published it, without errata" : "The RFCXML as the IETF archive has it",
              "aria-label": rfc ? "Published RFCXML at rfc-editor.org" : "Published RFCXML at ietf.org",
            },
            "↗",
          )
        : null,
    );
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
      h("div", { class: "input-group input-group-sm mb-2" }, target, h("button", { class: "btn btn-outline-primary", type: "submit" }, "Open")),
      h(
        "div",
        { class: "d-flex flex-wrap gap-2 align-items-center" },
        h("button", { class: "btn btn-sm btn-primary", type: "button", onclick: () => void runCheck(active) }, "Check"),
        h(
          "button",
          {
            class: "btn btn-sm btn-outline-secondary",
            type: "button",
            title: "Upload a text file: .txt (RFC or draft), .xml (xml2rfc), .md or .markdown (kramdown-rfc), .asn or .asn1 (bare ASN.1). Up to 8 MB. You can also drop files onto the editor.",
            onclick: () => fileInput.click(),
          },
          "Upload…",
        ),
        fileInput,
        formatSelect,
        h("div", { class: "form-check mb-0" }, snippets, h("label", { class: "form-check-label small", for: "opt-snippets" }, "Snippets")),
        h("div", { class: "form-check mb-0" }, auto, h("label", { class: "form-check-label small", for: "opt-auto" }, "Auto-check")),
        viewLinks,
        status,
      ),
    ),
    strip.element,
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
          tabButton("pane-errata", "Applied Errata", false, errataBadge),
        ),
        h(
          "div",
          { class: "tab-content pane-scroll" },
          tabPane("pane-diagnostics", true, diagnosticsPane),
          tabPane("pane-modules", false, modulesPane),
          tabPane("pane-imports", false, importsPane),
          tabPane("pane-errata", false, errataPane),
        ),
      ),
    ),
  );
  root.hidden = false;
  editor = createEditor(editorHost, {
    highlighting: false,
    onChange: () => {
      if (!programmatic) active.scheduleAuto();
    },
    complete: (text, position, signal) => api.complete(text, active.name, selectedFormat(active), position, signal),
  });
  root.hidden = true;
  active = createTab(editor.view.state);
  tabs = [active];
  refresh();

  return {
    show(route) {
      const wanted = targetOf(route);
      if (!wanted) return;
      if (wanted === handledHash) {
        handledHash = "";
        return;
      }
      // Coming back to a link (the back button, a bookmark) shows the tab that already has it.
      openRoute(route);
    },
  };
}
