import { type CorpusResponse, type ImportInfo, type ModuleDetail, type ModuleListItem, type ModulePage, type OidAssignment, api, documentTitle, moduleLabel, rfcNumber } from "./api";
import { createEditor } from "./editor";
import { debounce, h, replace } from "./dom";
import { parseOid } from "./oid";
import { OffsetMap } from "./offsets";
import { corpusHash, moduleHash, oidHash, rfcModuleHash } from "./router";

const STATUS_BADGE: Record<ModuleListItem["status"], string> = {
  clean: "text-bg-success",
  warnings: "text-bg-warning",
  errors: "text-bg-danger",
  unparseable: "text-bg-secondary",
};

export function statusBadge(status: ModuleListItem["status"]): HTMLElement {
  return h("span", { class: `badge ${STATUS_BADGE[status]}` }, status);
}

function message(text: string, kind = "text-secondary"): HTMLElement {
  return h("div", { class: `p-3 ${kind}` }, text);
}

const PAGE_SIZE = 50;

/** An RFC module opens in the editor at its place in the RFC; modules of other documents have no text to open it in. */
function moduleLink(m: ModuleListItem): string {
  const rfc = m.body === "rfc" ? rfcNumber(m.document) : null;
  return rfc !== null ? rfcModuleHash(rfc, m.name, m.oid) : moduleHash(m.document, m.name, undefined, m.oid);
}

export interface CorpusBrowser {
  show(query: string): void;
}

export function createCorpusBrowser(root: HTMLElement): CorpusBrowser {
  root.className = "view view-page";
  const summary = h("div", { class: "mb-3 small text-secondary" });
  const results = h("div");
  let controller: AbortController | null = null;
  let current = "\u0000";
  let offset = 0;
  const input = h("input", {
    class: "form-control",
    type: "search",
    placeholder: "Search by module name or OID, for example PKIX1 or 1.3.6.1.5.5.7",
    "aria-label": "Search modules",
  });

  async function search(query: string): Promise<void> {
    controller?.abort();
    const run = new AbortController();
    controller = run;
    try {
      const page = await api.modules(query, offset, PAGE_SIZE, run.signal);
      if (run.signal.aborted) return;
      if (!Array.isArray(page?.modules)) {
        replace(results, message("The server answered in an older format than this page expects. Restart glint-server so it matches the web files.", "text-danger"));
        return;
      }
      renderResults(page);
    } catch (error) {
      if (error instanceof DOMException && error.name === "AbortError") return;
      replace(results, message(error instanceof Error ? error.message : String(error), "text-danger"));
    }
  }

  function goTo(newOffset: number): void {
    offset = Math.max(0, newOffset);
    void search(current);
    window.scrollTo({ top: 0 });
  }

  function pager(page: ModulePage): HTMLElement {
    const first = page.offset + 1;
    const last = page.offset + page.modules.length;
    const previous = h("button", { class: "btn btn-sm btn-outline-secondary", type: "button", onclick: () => goTo(page.offset - page.limit) }, "Previous");
    const next = h("button", { class: "btn btn-sm btn-outline-secondary", type: "button", onclick: () => goTo(page.offset + page.limit) }, "Next");
    if (page.offset === 0) previous.disabled = true;
    if (last >= page.total) next.disabled = true;
    return h(
      "div",
      { class: "d-flex align-items-center gap-2 my-2 small" },
      previous,
      next,
      h("span", { class: "text-secondary" }, `${first}–${last} of ${page.total}`),
    );
  }

  function renderResults(page: ModulePage): void {
    const items = page.modules;
    if (items.length === 0) {
      if (page.total > 0) {
        goTo(0);
        return;
      }
      replace(results, message("No modules match."));
      return;
    }
    replace(
      results,
      pager(page),
      h(
        "table",
        { class: "table table-sm table-hover align-middle" },
        h(
          "thead",
          {},
          h("tr", {}, h("th", {}, "Module"), h("th", {}, "Document"), h("th", {}, "Section"), h("th", {}, "OID"), h("th", {}, "Status")),
        ),
        h(
          "tbody",
          {},
          ...items.map((m) =>
            h(
              "tr",
              {},
              h("td", {}, h("a", { href: moduleLink(m) }, m.name)),
              h(
                "td",
                {},
                documentTitle(m.document, m.edition),
                m.obsoleted_by.length > 0 ? h("span", { class: "badge text-bg-light ms-1" }, `obsoleted by RFC ${m.obsoleted_by.join(", ")}`) : null,
              ),
              h("td", {}, m.section ?? "—"),
              h("td", { class: "font-monospace small" }, m.oid ? h("a", { href: oidHash(m.oid) }, m.oid) : "—"),
              h("td", {}, statusBadge(m.status)),
            ),
          ),
        ),
      ),
      pager(page),
    );
  }

  function renderSummary(corpus: CorpusResponse): void {
    const parts = Object.entries(corpus.by_status).map(([status, count]) => `${count} ${status}`);
    replace(summary, `Corpus ${corpus.version}: ${corpus.modules} modules (${parts.join(", ")}), ${corpus.oid_assignments} OID assignments.`);
  }

  const onInput = debounce(() => {
    const query = input.value.trim();
    current = query;
    offset = 0;
    history.replaceState(null, "", corpusHash(query));
    void search(query);
  }, 250);
  input.addEventListener("input", onInput);

  replace(root, h("div", { class: "container-lg py-3" }, h("h1", { class: "h4" }, "Corpus"), summary, h("div", { class: "mb-3" }, input), results));
  api.corpus().then(renderSummary, () => replace(summary, "The corpus summary is not available."));

  return {
    show(query) {
      if (query === current) return;
      current = query;
      offset = 0;
      input.value = query;
      void search(query);
    },
  };
}

export interface ModuleViewer {
  show(source: string, name: string, from?: number, to?: number, oid?: string): void;
}

export function createModuleViewer(root: HTMLElement): ModuleViewer {
  root.className = "view view-page";
  const header = h("div");
  const importsBox = h("div");
  const editorHost = h("div", { class: "module-editor border rounded" });
  replace(root, h("div", { class: "container-lg py-3" }, header, importsBox, editorHost));
  root.hidden = false;
  const editor = createEditor(editorHost, { readOnly: true, doc: "" });
  root.hidden = true;
  let loaded = "";
  let offsets: OffsetMap | null = null;
  let requested = 0;

  function reveal(from?: number, to?: number): void {
    if (from === undefined || !offsets || Number.isNaN(from)) return;
    const end = to === undefined || Number.isNaN(to) ? from : to;
    editor.reveal(offsets.toIndex(from), offsets.toIndex(end));
  }

  function renderImports(imports: ImportInfo[]): void {
    if (imports.length === 0) {
      replace(importsBox);
      return;
    }
    replace(
      importsBox,
      h(
        "div",
        { class: "mb-3" },
        h("div", { class: "fw-semibold small mb-1" }, "Imports"),
        h(
          "ul",
          { class: "list-unstyled small mb-0" },
          ...imports.map((i) =>
            h(
              "li",
              {},
              i.symbols.join(", "),
              " from ",
              i.target
                ? h("a", { href: moduleHash(i.target.source, i.target.name, undefined, i.target.oid) }, moduleLabel(i.target))
                : h("span", { class: "text-warning-emphasis" }, `${i.module} (not resolved)`),
            ),
          ),
        ),
      ),
    );
  }

  function renderHeader(m: ModuleDetail): void {
    const rfc = m.body === "rfc" ? rfcNumber(m.document) : null;
    const title = documentTitle(m.document, m.edition);
    const href = rfc !== null ? `https://www.rfc-editor.org/rfc/rfc${rfc}` : m.url;
    const rfcLink = href ? h("a", { href, rel: "noopener" }, title) : title;
    replace(
      header,
      h("h1", { class: "h4 mb-1" }, m.name),
      h(
        "div",
        { class: "small text-secondary mb-3 d-flex flex-wrap gap-3 align-items-center" },
        h("span", {}, rfcLink, m.section ? `, ${m.section}` : ""),
        m.oid ? h("a", { class: "font-monospace", href: oidHash(m.oid) }, m.oid) : null,
        statusBadge(m.status),
        h("span", {}, `syntax ${m.syntax_era}`),
        m.rfc_status ? h("span", {}, m.rfc_status) : null,
        m.obsoleted_by.length > 0 ? h("span", {}, `obsoleted by RFC ${m.obsoleted_by.join(", ")}`) : null,
        m.source_kind.kind === "overridden" ? h("span", { class: "badge text-bg-info" }, m.source_kind.kind) : null,
      ),
    );
  }

  return {
    show(source, name, from, to, oid) {
      const key = `${source}/${name}/${oid ?? ""}`;
      if (key === loaded) {
        reveal(from, to);
        return;
      }
      loaded = key;
      const mine = ++requested;
      replace(header, message("Loading…"));
      replace(importsBox);
      api.module(source, name, oid).then(
        (module) => {
          if (mine !== requested) return;
          renderHeader(module);
          renderImports(module.imports);
          editor.setText(module.text);
          offsets = new OffsetMap(editor.text());
          reveal(from, to);
        },
        (error: unknown) => {
          if (mine !== requested) return;
          loaded = "";
          replace(header, message(error instanceof Error ? error.message : String(error), "text-danger"));
        },
      );
    },
  };
}

const EXAMPLE_OID = "1.3.6.1.5.5.7";

export interface OidPage {
  show(oid?: string): void;
}

export function createOidPage(root: HTMLElement): OidPage {
  root.className = "view view-page";
  const input = h("input", {
    class: "form-control font-monospace",
    type: "text",
    placeholder: EXAMPLE_OID,
    title: "Dotted digits, or names with numbers in parentheses such as iso(1) identified-organization(3) dod(6)",
    "aria-label": "Object identifier",
  });
  const notes = h("div", { class: "mt-2" });
  const output = h("div", { class: "mt-3" });
  let loaded = "\u0000";

  function showNotes(warnings: string[]): void {
    replace(notes, ...warnings.map((w) => h("div", { class: "small text-warning-emphasis" }, w)));
  }

  /** An empty box looks up the example shown as its placeholder; text that is not an OID says so. */
  function submit(): void {
    const parsed = parseOid(input.value.trim() === "" ? EXAMPLE_OID : input.value);
    if (!parsed) {
      showNotes([]);
      loaded = "\u0000";
      replace(output, message("That is not an OID. Use dotted digits or names with numbers in parentheses, such as iso(1) identified-organization(3).", "text-danger"));
      return;
    }
    showNotes(parsed.warnings);
    const hash = oidHash(parsed.oid);
    // Looking up what is already shown does not change the hash, so there is no event to wait for.
    if (window.location.hash === hash) void lookup(parsed.oid);
    else window.location.hash = hash;
  }

  function table(rows: OidAssignment[]): HTMLElement {
    return h(
      "table",
      { class: "table table-sm align-middle" },
      h("thead", {}, h("tr", {}, h("th", {}, "OID"), h("th", {}, "Name"), h("th", {}, "Module"), h("th", {}, "Document"))),
      h(
        "tbody",
        {},
        ...rows.map((a) =>
          h(
            "tr",
            {},
            h("td", { class: "font-monospace small" }, h("a", { href: oidHash(a.oid) }, a.oid)),
            h("td", {}, a.name),
            h("td", {}, h("a", { href: moduleHash(a.document, a.module) }, a.module)),
            h("td", {}, documentTitle(a.document)),
          ),
        ),
      ),
    );
  }

  async function lookup(oid: string): Promise<void> {
    loaded = oid;
    replace(output, message("Loading…"));
    try {
      const result = await api.oid(oid);
      if (loaded !== oid) return;
      replace(
        output,
        h(
          "div",
          { class: "d-flex flex-wrap gap-3 mb-3 small" },
          h("span", { class: "font-monospace fw-semibold" }, result.oid),
          result.parent ? h("a", { href: oidHash(result.parent) }, "parent") : null,
          result.module ? h("a", { href: moduleHash(result.module.source, result.module.name, undefined, result.module.oid) }, `module ${result.module.name}`) : null,
        ),
        result.assignments.length > 0 ? h("div", {}, h("h2", { class: "h6" }, "Assigned as"), table(result.assignments)) : message("No assignment of this exact OID in the corpus."),
        result.children.length > 0
          ? h(
              "div",
              {},
              h("h2", { class: "h6" }, "Below this OID"),
              table(result.children),
              result.truncated ? h("div", { class: "small text-secondary" }, "The list is cut off; look up a child to see more.") : null,
            )
          : null,
      );
    } catch (error) {
      if (loaded !== oid) return;
      replace(output, message(error instanceof Error ? error.message : String(error), "text-danger"));
    }
  }

  replace(
    root,
    h(
      "div",
      { class: "container-lg py-3" },
      h("h1", { class: "h4" }, "OID lookup"),
      h(
        "form",
        {
          class: "input-group",
          onsubmit: (event) => {
            event.preventDefault();
            submit();
          },
        },
        input,
        h("button", { class: "btn btn-primary", type: "submit" }, "Look up"),
      ),
      notes,
      output,
    ),
  );

  return {
    show(oid) {
      if (!oid) {
        loaded = "\u0000";
        replace(output);
        showNotes([]);
        return;
      }
      if (oid !== loaded) {
        input.value = oid;
        void lookup(oid);
      }
    },
  };
}
