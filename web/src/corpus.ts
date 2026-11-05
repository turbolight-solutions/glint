import { type CorpusResponse, type ModuleListItem, type OidAssignment, api, documentTitle } from "./api";
import { debounce, h, replace } from "./dom";
import { type Sort, type SortKey, formatSort, nextSort, sortModules } from "./modulesort";
import { parseOid } from "./oid";
import { corpusHash, moduleHash, oidHash } from "./router";

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

function formatSortOrNone(sort: Sort | null): string {
  return sort ? formatSort(sort) : "";
}
/** The most one answer holds. The whole corpus is a few hundred modules, so one or two requests load it. */
const FETCH_SIZE = 500;

const COLUMNS: { key: SortKey; label: string }[] = [
  { key: "name", label: "Module" },
  { key: "document", label: "Document" },
  { key: "section", label: "Section" },
  { key: "oid", label: "OID" },
  { key: "status", label: "Status" },
];

export interface CorpusBrowser {
  show(query: string, sort?: Sort | null): void;
}

/** Every module matching `query`, in order of relevance. */
async function fetchAll(query: string, signal: AbortSignal): Promise<ModuleListItem[]> {
  const first = await api.modules(query, 0, FETCH_SIZE, signal);
  const modules = [...first.modules];
  while (modules.length < first.total) {
    const page = await api.modules(query, modules.length, FETCH_SIZE, signal);
    if (page.modules.length === 0) break;
    modules.push(...page.modules);
  }
  return modules;
}

export function createCorpusBrowser(root: HTMLElement): CorpusBrowser {
  root.className = "view view-page";
  const summary = h("div", { class: "mb-3 small text-secondary" });
  const results = h("div");
  let controller: AbortController | null = null;
  let current = "\u0000";
  let offset = 0;
  let sort: Sort | null = null;
  let loading = false;
  let matches: ModuleListItem[] = [];
  let sorted: ModuleListItem[] = [];
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
    loading = true;
    try {
      const modules = await fetchAll(query, run.signal);
      if (run.signal.aborted) return;
      loading = false;
      matches = modules;
      sorted = sortModules(matches, sort);
      renderResults();
    } catch (error) {
      if (error instanceof DOMException && error.name === "AbortError") return;
      loading = false;
      replace(results, message(error instanceof Error ? error.message : String(error), "text-danger"));
    }
  }

  function goTo(newOffset: number): void {
    const last = Math.max(0, Math.floor((sorted.length - 1) / PAGE_SIZE) * PAGE_SIZE);
    offset = Math.min(Math.max(0, newOffset), last);
    renderResults();
    window.scrollTo({ top: 0 });
  }

  function sortBy(key: SortKey): void {
    applySort(nextSort(sort, key));
    history.replaceState(null, "", corpusHash(current, sort));
  }

  function applySort(newSort: Sort | null): void {
    sort = newSort;
    sorted = sortModules(matches, sort);
    offset = 0;
    if (!loading) renderResults();
  }

  function pager(): HTMLElement {
    const first = offset + 1;
    const last = Math.min(offset + PAGE_SIZE, sorted.length);
    const previous = h("button", { class: "btn btn-sm btn-outline-secondary", type: "button", onclick: () => goTo(offset - PAGE_SIZE) }, "Previous");
    const next = h("button", { class: "btn btn-sm btn-outline-secondary", type: "button", onclick: () => goTo(offset + PAGE_SIZE) }, "Next");
    if (offset === 0) previous.disabled = true;
    if (last >= sorted.length) next.disabled = true;
    return h(
      "div",
      { class: "d-flex align-items-center gap-2 my-2 small" },
      previous,
      next,
      h("span", { class: "text-secondary" }, `${first}–${last} of ${sorted.length}`),
    );
  }

  function header(column: { key: SortKey; label: string }): HTMLElement {
    const active = sort?.key === column.key ? sort.direction : null;
    const arrow = active === "asc" ? " \u25B2" : active === "desc" ? " \u25BC" : "";
    return h(
      "th",
      { "aria-sort": active === "asc" ? "ascending" : active === "desc" ? "descending" : undefined },
      h(
        "button",
        {
          class: `sort-button${active ? " active" : ""}`,
          type: "button",
          title: column.key === "status" ? "Sort by severity" : `Sort by ${column.label.toLowerCase()}`,
          onclick: () => sortBy(column.key),
        },
        column.label + arrow,
      ),
    );
  }

  function renderResults(): void {
    if (sorted.length === 0) {
      replace(results, message("No modules match."));
      return;
    }
    if (offset >= sorted.length) offset = 0;
    const items = sorted.slice(offset, offset + PAGE_SIZE);
    replace(
      results,
      pager(),
      h(
        "table",
        { class: "table table-sm table-hover align-middle" },
        h("thead", {}, h("tr", {}, ...COLUMNS.map(header))),
        h(
          "tbody",
          {},
          ...items.map((m) =>
            h(
              "tr",
              {},
              h("td", {}, h("a", { href: moduleHash(m.document, m.name, m.oid) }, m.name)),
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
      pager(),
    );
  }

  function renderSummary(corpus: CorpusResponse): void {
    const parts = Object.entries(corpus.by_status).map(([status, count]) => `${count} ${status}`);
    replace(
      summary,
      `Data ${corpus.build}: ${corpus.modules} modules from ${corpus.documents} documents (${parts.join(", ")}), ${corpus.oid_assignments} OID assignments, ${corpus.oid_nodes} labelled arcs.`,
    );
  }

  const onInput = debounce(() => {
    const query = input.value.trim();
    current = query;
    offset = 0;
    history.replaceState(null, "", corpusHash(query, sort));
    void search(query);
  }, 250);
  input.addEventListener("input", onInput);

  replace(root, h("div", { class: "container-lg py-3" }, h("h1", { class: "h4" }, "Corpus"), summary, h("div", { class: "mb-3" }, input), results));
  api.corpus().then(renderSummary, () => replace(summary, "The corpus summary is not available."));

  return {
    show(query, newSort = null) {
      if (query === current) {
        if (formatSortOrNone(newSort) !== formatSortOrNone(sort)) applySort(newSort);
        return;
      }
      sort = newSort;
      current = query;
      offset = 0;
      input.value = query;
      void search(query);
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
          result.module ? h("a", { href: moduleHash(result.module.source, result.module.name, result.module.oid) }, `module ${result.module.name}`) : null,
        ),
        result.allowlisted ? h("div", { class: "alert alert-secondary small py-2" }, `On the allowlist of wrong but deployed OIDs: ${result.allowlisted}`) : null,
        result.labels.length > 0
          ? h(
              "div",
              { class: "mb-3" },
              h("h2", { class: "h6" }, "Labelled as"),
              h(
                "ul",
                { class: "small mb-0" },
                ...result.labels.map((l) =>
                  h(
                    "li",
                    {},
                    h("span", { class: "font-monospace" }, `${l.label}(${result.oid.split(".").pop() ?? ""})`),
                    ` in ${l.documents.slice(0, 5).map((d) => documentTitle(d)).join(", ")}${l.documents.length > 5 ? ` and ${l.documents.length - 5} more` : ""}`,
                  ),
                ),
              ),
            )
          : null,
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
