// @vitest-environment jsdom
import { EditorView } from "@codemirror/view";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { ApiError, type CheckResponse, type ModuleInfo, setBackend } from "./api";
import { createChecker } from "./checker";
import { useBackend } from "./test-backend";

const RESPONSE: CheckResponse = {
  schema_version: 1,
  glint_version: "0.1.0",
  data_build: "2026-11-05.0123abcd",
  document: { name: "x.asn", format: "bare" },
  files: [
    { id: 0, name: "x.asn", kind: "document" },
    { id: 1, name: "rfc1000#Base", kind: "corpus", module: { source: "rfc1000", name: "Base" } },
  ],
  blocks: [{ kind: "module", section: null, title: null, start: 0, end: 20, line: 1, column: 1 }],
  modules: [
    {
      name: "Thing",
      oid: null,
      section: null,
      start: 0,
      end: 20,
      imports: [
        {
          module: "Base",
          resolved_to: "rfc1000#Base",
          target: { source: "rfc1000", name: "Base" },
          symbols: ["Name"],
        },
      ],
    },
  ],
  diagnostics: [
    {
      code: "E300",
      severity: "error",
      message: "differs from its definition",
      primary: { file: 0, span: { start: 2, end: 5 }, line_col: [1, 3], section: "4.1", text: null },
      secondary: [{ file: 1, span: { start: 10, end: 14 }, line_col: [2, 1], section: null, text: "defined here" }],
      notes: [],
      help: null,
    },
  ],
};

function mount(): HTMLElement {
  document.body.innerHTML = '<section id="root"></section>';
  return document.getElementById("root") as HTMLElement;
}

function checkButton(root: HTMLElement): HTMLButtonElement | undefined {
  return [...root.querySelectorAll("button")].find((b) => b.textContent === "Check");
}

/** Answers every call with `body`. */
function answerWith(body: unknown) {
  return useBackend(() => body);
}

/** The CodeMirror view of the document being checked. */
function mainView(root: HTMLElement): EditorView {
  return EditorView.findFromDOM(root.querySelector<HTMLElement>(".editor-host .cm-editor") as HTMLElement) as EditorView;
}

// jsdom does no layout; CodeMirror measures text through Range rects, so give it empty ones.
beforeAll(() => {
  const none = { length: 0, item: () => null, [Symbol.iterator]: () => [][Symbol.iterator]() };
  Range.prototype.getClientRects ??= () => none as unknown as DOMRectList;
  Range.prototype.getBoundingClientRect ??= () => new DOMRect();
});

afterEach(() => {
  setBackend(null);
  vi.unstubAllGlobals();
});

describe("checker view", () => {
  it("renders diagnostics, modules and import links after a check", async () => {
    const call = answerWith(RESPONSE);
    const root = mount();
    createChecker(root);
    checkButton(root)?.click();
    await vi.waitFor(() => expect(root.textContent).toContain("differs from its definition"));
    expect(root.textContent).toContain("1 error");
    expect(root.textContent).toContain("Glint 0.1.0, data 2026-11-05.0123abcd");
    expect(root.querySelector('a[href="#rfc1000?module=Base"]')).not.toBeNull();
    expect(call.mock.calls[0]?.[0]).toBe("check");
  });

  it("shows an error instead of throwing", async () => {
    useBackend(() => {
      throw new ApiError("not_found", "draft-x-y is not in the corpus");
    });
    const root = mount();
    createChecker(root);
    checkButton(root)?.click();
    await vi.waitFor(() => expect(root.textContent).toContain("draft-x-y is not in the corpus"));
  });

  it("fetches a draft named in the route and fills the editor", async () => {
    const withContent = {
      ...RESPONSE,
      document: { name: "draft-a-b-01", format: "bare" as const, content: "Thing ::= INTEGER" },
    };
    const call = answerWith(withContent);
    const root = mount();
    const checker = createChecker(root);
    checker.show({ view: "checker", draft: "draft-a-b-01" });
    await vi.waitFor(() => expect(root.textContent).toContain("Thing ::= INTEGER"));
    const [method, args] = call.mock.calls[0] as unknown as [string, unknown[]];
    expect(method).toBe("checkDocument");
    expect(args[0]).toMatchObject({ draft: "draft-a-b-01" });
  });

  it("links an import and a related location to the module of the OID the check named", async () => {
    const oid = "1.2.3.2";
    answerWith({
      ...RESPONSE,
      files: [
        RESPONSE.files[0],
        { id: 1, name: "rfc4998#ERS@1.2.3.2", kind: "corpus", module: { source: "rfc4998", name: "ERS", oid } },
      ],
      modules: [
        {
          name: "Thing",
          oid: null,
          section: null,
          start: 0,
          end: 20,
          imports: [{ module: "ERS", resolved_to: "rfc4998#ERS@1.2.3.2", target: { source: "rfc4998", name: "ERS", oid }, symbols: ["X"] }],
        },
      ],
    });
    const root = mount();
    createChecker(root);
    checkButton(root)?.click();
    await vi.waitFor(() => expect(root.textContent).toContain("differs from its definition"));
    const links = [...root.querySelectorAll('a[href="#rfc4998?module=ERS&oid=1.2.3.2"]')].map((a) => a.textContent);
    expect(links).toContain("rfc4998#ERS (1.2.3.2)");
    expect(links).toContain("defined here — rfc4998#ERS@1.2.3.2:2:1");
  });

  it("links an import resolved in the document itself to that module in the editor", async () => {
    const content = "User DEFINITIONS ::= BEGIN END\nCommon DEFINITIONS ::= BEGIN END\n";
    const common = content.indexOf("Common");
    const module = (name: string, start: number, imports: ModuleInfo["imports"]): ModuleInfo => ({
      name,
      oid: null,
      section: null,
      start,
      end: start + 30,
      imports,
    });
    answerWith({
      ...RESPONSE,
      document: { name: "draft-a-b-01", format: "text" as const, content },
      diagnostics: [],
      modules: [
        module("User", 0, [
          { module: "Common", resolved_to: "Common", target: null, symbols: ["X"] },
          { module: "Elsewhere", resolved_to: "Elsewhere", target: null, symbols: ["Y"] },
        ]),
        module("Common", common, []),
      ],
    });
    const root = mount();
    createChecker(root).show({ view: "checker", draft: "draft-a-b-01" });
    await vi.waitFor(() => expect(mainView(root).state.doc.toString()).toBe(content));
    const pane = root.querySelector("#pane-imports") as HTMLElement;
    const link = [...pane.querySelectorAll("a")].find((a) => a.textContent === "Common (this document)");
    expect(link).toBeDefined();
    link?.click();
    expect(mainView(root).state.selection.main.from).toBe(common);
    // A name the document has no module of (a built-in) stays plain text.
    expect(pane.textContent).toContain("Elsewhere (this document)");
    expect([...pane.querySelectorAll("a")].some((a) => a.textContent === "Elsewhere (this document)")).toBe(false);
  });
});

describe("checker tabs", () => {
  function stubFetch(): void {
    useBackend((_method, args) => {
      const body = args[0] as { draft?: string; rfc?: number };
      const name = String(body.draft ?? `rfc${body.rfc}`);
      const document = { name, format: "bare" as const, content: `-- body of ${name}\nThing ::= INTEGER` };
      const diagnostics = name === "draft-a-b-01" ? RESPONSE.diagnostics : [];
      return { ...RESPONSE, document, diagnostics };
    });
  }

  const titles = (root: HTMLElement) => [...root.querySelectorAll(".doc-tab-title")].map((t) => t.textContent);
  const tab = (root: HTMLElement, index: number) =>
    root.querySelectorAll<HTMLElement>(".doc-tab-label")[index] as HTMLElement;
  const menu = (root: HTMLElement, index: number, item: string) => {
    tab(root, index).dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 5, clientY: 5 }));
    [...document.querySelectorAll<HTMLButtonElement>(".tab-menu button")].find((b) => b.textContent === item)?.click();
  };
  const route = (draft: string) => ({ view: "checker" as const, draft });

  async function open(root: HTMLElement, checker: ReturnType<typeof createChecker>, draft: string) {
    checker.show(route(draft));
    await vi.waitFor(() => expect(root.textContent).toContain(`body of ${draft}`));
  }

  it("starts with one empty tab", () => {
    stubFetch();
    const root = mount();
    createChecker(root);
    expect(titles(root)).toEqual(["Untitled"]);
  });

  it("reuses an empty tab for the first fetch, then opens a new tab for each one", async () => {
    stubFetch();
    const root = mount();
    const checker = createChecker(root);
    await open(root, checker, "draft-a-b-01");
    expect(titles(root)).toEqual(["draft-a-b-01"]);
    await open(root, checker, "draft-c-d-02");
    expect(titles(root)).toEqual(["draft-a-b-01", "draft-c-d-02"]);
    expect(root.textContent).not.toContain("body of draft-a-b-01");
  });

  it("gives each fetch of the same document its own tab when asked to fetch again", async () => {
    stubFetch();
    const root = mount();
    const checker = createChecker(root);
    await open(root, checker, "draft-a-b-01");
    const input = root.querySelector<HTMLInputElement>('input[type="text"]') as HTMLInputElement;
    input.value = "draft-a-b-01";
    root.querySelector("form")?.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    await vi.waitFor(() => expect(titles(root)).toHaveLength(2));
  });

  it("switches back to a tab with its text and findings", async () => {
    stubFetch();
    const root = mount();
    const checker = createChecker(root);
    await open(root, checker, "draft-a-b-01");
    expect(root.textContent).toContain("differs from its definition");
    await open(root, checker, "draft-c-d-02");
    expect(root.textContent).toContain("No findings.");
    expect(root.textContent).not.toContain("differs from its definition");

    tab(root, 0).click();
    expect(root.textContent).toContain("body of draft-a-b-01");
    expect(root.textContent).toContain("differs from its definition");
    expect(root.textContent).toContain("1 error");
    expect(root.querySelector<HTMLInputElement>('input[type="text"]')?.value).toBe("draft-a-b-01");
  });

  it("shows a fetch that finishes in the background when its tab is selected", async () => {
    stubFetch();
    const root = mount();
    const checker = createChecker(root);
    await open(root, checker, "draft-a-b-01");
    checker.show(route("draft-c-d-02"));
    tab(root, 0).click();
    await vi.waitFor(() => expect(root.querySelector(".spinner-border")).toBeNull());
    expect(root.textContent).toContain("body of draft-a-b-01");
    tab(root, 1).click();
    expect(root.textContent).toContain("body of draft-c-d-02");
  });

  it("closes a tab, close others and close all from the context menu", async () => {
    stubFetch();
    const root = mount();
    const checker = createChecker(root);
    await open(root, checker, "draft-a-b-01");
    await open(root, checker, "draft-c-d-02");
    await open(root, checker, "draft-e-f-03");
    expect(titles(root)).toHaveLength(3);

    menu(root, 2, "Close");
    expect(titles(root)).toEqual(["draft-a-b-01", "draft-c-d-02"]);
    expect(root.textContent).toContain("body of draft-c-d-02");

    menu(root, 0, "Close others");
    expect(titles(root)).toEqual(["draft-a-b-01"]);
    expect(root.textContent).toContain("body of draft-a-b-01");

    await open(root, checker, "draft-g-h-04");
    menu(root, 0, "Close all");
    expect(titles(root)).toEqual(["Untitled"]);
    expect(root.textContent).not.toContain("body of");
  });

  it("opens a new empty tab with the plus button", async () => {
    stubFetch();
    const root = mount();
    const checker = createChecker(root);
    await open(root, checker, "draft-a-b-01");
    root.querySelector<HTMLElement>(".doc-tab-add")?.click();
    expect(titles(root)).toEqual(["draft-a-b-01", "Untitled"]);
    // The new tab is empty, so the next fetch goes into it.
    await open(root, checker, "draft-c-d-02");
    expect(titles(root)).toEqual(["draft-a-b-01", "draft-c-d-02"]);
  });

  it("opens each uploaded file in its own tab", async () => {
    stubFetch();
    const root = mount();
    createChecker(root);
    const input = root.querySelector<HTMLInputElement>('input[type="file"]') as HTMLInputElement;
    const files = [new File(["A ::= INTEGER"], "a.asn"), new File(["B ::= INTEGER"], "b.asn")];
    Object.defineProperty(input, "files", { value: files, configurable: true });
    input.dispatchEvent(new Event("change"));
    await vi.waitFor(() => expect(titles(root)).toEqual(["a.asn", "b.asn"]));
    expect(root.textContent).toContain("B ::= INTEGER");
    tab(root, 0).click();
    expect(root.textContent).toContain("A ::= INTEGER");
  });
});

describe("module panels and module links", () => {
  interface Part {
    name: string;
    oid?: string;
    section?: string;
    text: string;
  }

  /** A document of one module per part, separated by blank lines, with each module's span worked out. */
  function documentOf(parts: Part[]): { response: CheckResponse; starts: number[] } {
    let content = "";
    const starts: number[] = [];
    const modules: ModuleInfo[] = [];
    for (const part of parts) {
      if (content) content += "\n";
      starts.push(content.length);
      const start = content.length;
      content += part.text;
      modules.push({ name: part.name, oid: part.oid ?? null, section: part.section ?? null, start, end: content.length, imports: [] });
    }
    const response: CheckResponse = {
      ...RESPONSE,
      document: { name: "rfc1000", format: "bare", content },
      diagnostics: [],
      modules,
    };
    return { response, starts };
  }

  const FIRST = "First DEFINITIONS ::= BEGIN\nA ::= INTEGER\nEND\n";
  const SECOND = "Second DEFINITIONS ::= BEGIN\nB ::= BOOLEAN\nEND\n";
  const TWO = documentOf([
    { name: "First", text: FIRST },
    { name: "Second", section: "2", text: SECOND },
  ]);
  const secondStart = TWO.starts[1] as number;

  // RFC 4998 defines `ERS` twice, for two syntaxes, with different OIDs.
  const OLD_OID = "1.2.3.1";
  const NEW_OID = "1.2.3.2";
  const TWINS = documentOf([
    { name: "ERS", oid: OLD_OID, text: "ERS DEFINITIONS ::= BEGIN\nOld ::= INTEGER\nEND\n" },
    { name: "ERS", oid: NEW_OID, text: "ERS DEFINITIONS ::= BEGIN\nNew ::= INTEGER\nEND\n" },
  ]);

  const panels = (root: HTMLElement) => [...root.querySelectorAll<HTMLDetailsElement>("details.module-panel")];
  const head = (root: HTMLElement) => mainView(root).state.selection.main.head;

  it("puts the cursor at the start of the module a link names", async () => {
    answerWith(TWO.response);
    const root = mount();
    createChecker(root).show({ view: "checker", rfc: 1000, module: "Second" });
    await vi.waitFor(() => expect(head(root)).toBe(secondStart));
  });

  it("moves the cursor when the same document is opened again for another module", async () => {
    answerWith(TWO.response);
    const root = mount();
    const checker = createChecker(root);
    checker.show({ view: "checker", rfc: 1000, module: "Second" });
    await vi.waitFor(() => expect(head(root)).toBe(secondStart));
    checker.show({ view: "checker", rfc: 1000, module: "First" });
    await vi.waitFor(() => expect(head(root)).toBe(0));
  });

  it("opens a module of another standards body in the checker and puts the cursor at its start", async () => {
    const text = "-- a comment\nItu1 DEFINITIONS ::= BEGIN\nA ::= INTEGER\nEND\n";
    const start = text.indexOf("Itu1");
    const response: CheckResponse = {
      ...RESPONSE,
      document: { name: "X.509#Itu1", format: "bare" },
      diagnostics: [],
      modules: [{ name: "Itu1", oid: null, section: null, start, end: text.length, imports: [] }],
    };
    const call = useBackend((method) => (method === "module" ? { name: "Itu1", text } : response));
    const root = mount();
    createChecker(root).show({ view: "checker", document: "X.509", module: "Itu1" });
    await vi.waitFor(() => expect(head(root)).toBe(start));
    expect(call.mock.calls[0]?.slice(0, 2)).toEqual(["module", ["X.509", "Itu1", null]]);
    expect(root.textContent).toContain("A ::= INTEGER");
  });

  describe("when a document has two modules of one name", () => {
    const [oldStart, newStart] = TWINS.starts as [number, number];

    it.each([
      ["the earlier", OLD_OID, oldStart],
      ["the later", NEW_OID, newStart],
    ])("puts the cursor at %s one when the link gives its OID", async (_which, oid, expected) => {
      answerWith(TWINS.response);
      const root = mount();
      createChecker(root).show({ view: "checker", rfc: 4998, module: "ERS", moduleOid: oid });
      await vi.waitFor(() => expect(head(root)).toBe(expected));
    });

    it("falls back to the first of that name when the link has no OID or one that matches none", async () => {
      answerWith(TWINS.response);
      const root = mount();
      const checker = createChecker(root);
      checker.show({ view: "checker", rfc: 4998, module: "ERS" });
      await vi.waitFor(() => expect(head(root)).toBe(oldStart));
      checker.show({ view: "checker", rfc: 4998, module: "ERS", moduleOid: NEW_OID });
      await vi.waitFor(() => expect(head(root)).toBe(newStart));
      checker.show({ view: "checker", rfc: 4998, module: "ERS", moduleOid: "9.9.9" });
      await vi.waitFor(() => expect(head(root)).toBe(oldStart));
    });

    it("lists both modules, each with its own panel", async () => {
      answerWith(TWINS.response);
      const root = mount();
      createChecker(root).show({ view: "checker", rfc: 4998 });
      await vi.waitFor(() => expect(panels(root)).toHaveLength(2));
      expect(panels(root).map((p) => p.textContent)).toEqual([expect.stringContaining(OLD_OID), expect.stringContaining(NEW_OID)]);
    });
  });

  it("lists a collapsed panel per module and builds its read-only view on opening", async () => {
    answerWith(TWO.response);
    const root = mount();
    createChecker(root).show({ view: "checker", rfc: 1000 });
    await vi.waitFor(() => expect(panels(root)).toHaveLength(2));
    expect(panels(root).every((p) => !p.open)).toBe(true);
    expect(root.querySelector(".module-panel-editor .cm-editor")).toBeNull();

    const second = panels(root)[1] as HTMLDetailsElement;
    second.open = true;
    await vi.waitFor(() => expect(second.querySelector(".cm-content")).not.toBeNull());
    const view = EditorView.findFromDOM(second.querySelector<HTMLElement>(".cm-editor") as HTMLElement) as EditorView;
    expect(view.state.doc.toString()).toBe(SECOND);
    expect(view.state.readOnly).toBe(true);
    expect(second.querySelector(".cm-lineNumbers")?.textContent).toContain("5");
  });

  it("keeps a panel open when the document is checked again", async () => {
    answerWith(TWO.response);
    const root = mount();
    createChecker(root).show({ view: "checker", rfc: 1000 });
    await vi.waitFor(() => expect(panels(root)).toHaveLength(2));
    (panels(root)[0] as HTMLDetailsElement).open = true;
    await vi.waitFor(() => expect(root.querySelectorAll(".module-panel-editor .cm-editor")).toHaveLength(1));
    checkButton(root)?.click();
    await vi.waitFor(() => expect(panels(root)[0]?.open).toBe(true));
    expect(root.querySelectorAll(".module-panel-editor .cm-editor")).toHaveLength(1);
    expect(panels(root)[1]?.open).toBe(false);
  });
});

describe("applied errata", () => {
  // Erratum 7 changed line 2 (hunk at lines 1-3), erratum 9 removed the comment that was before line 5.
  const TEXT = "one\nTWO\nthree\nfour\nsix\n";
  const ERRATA = [
    {
      id: 7,
      by: "human",
      note: "Line two is upper case",
      patch: "--- a/rfc1000.txt\n+++ b/rfc1000.txt\n@@ -1,3 +1,3 @@\n one\n-two\n+TWO\n three\n",
      hunks: [{ start_line: 1, end_line: 3 }],
    },
    {
      id: 9,
      by: "claude",
      note: "Line five goes",
      patch: "--- a/rfc1000.txt\n+++ b/rfc1000.txt\n@@ -5 +4,0 @@\n--- five\n",
      hunks: [{ start_line: 5, end_line: 4 }],
    },
  ];
  const withErrata = (document: Partial<CheckResponse["document"]>): CheckResponse => ({
    ...RESPONSE,
    diagnostics: [],
    document: { name: "rfc1000", format: "text", content: TEXT, errata: ERRATA, ...document },
  });
  const pane = (root: HTMLElement) => root.querySelector("#pane-errata") as HTMLElement;
  const badge = (root: HTMLElement) => root.querySelector("#pane-errata-tab .badge") as HTMLElement;

  async function openRfc(response: CheckResponse): Promise<HTMLElement> {
    answerWith(response);
    const root = mount();
    createChecker(root).show({ view: "checker", rfc: 1000 });
    await vi.waitFor(() => expect(root.textContent).toContain("No findings."));
    return root;
  }

  it("lists each erratum with its link, decider, note and coloured diff", async () => {
    const root = await openRfc(withErrata({}));
    expect(root.querySelector("#pane-errata-tab")?.textContent).toContain("Applied Errata");
    expect(badge(root).textContent).toBe("2");
    expect(badge(root).classList.contains("d-none")).toBe(false);
    const link = pane(root).querySelector<HTMLAnchorElement>('a[href="https://www.rfc-editor.org/errata/eid7"]');
    expect(link?.textContent).toBe("Erratum 7");
    expect(link?.target).toBe("_blank");
    expect(link?.rel).toBe("noopener");
    expect(pane(root).textContent).toContain("decided by human");
    expect(pane(root).textContent).toContain("Line two is upper case");
    expect(pane(root).textContent).toContain("decided by claude");
    const text = (selector: string) => [...pane(root).querySelectorAll(selector)].map((e) => e.textContent);
    expect(text(".diff-del")).toEqual(["-two", "--- five"]);
    expect(text(".diff-add")).toEqual(["+TWO"]);
    expect(text(".diff-ctx")).toEqual([" one", " three"]);
    expect(text(".errata-hunk")).toEqual(["lines 1–3", "removed before line 5"]);
    expect(pane(root).textContent).not.toContain("@@");
    expect(pane(root).textContent).not.toContain("+++");
  });

  it("selects a hunk's lines in the editor when the hunk or one of its lines is clicked", async () => {
    const root = await openRfc(withErrata({}));
    const view = mainView(root);
    const [first, removal] = [...pane(root).querySelectorAll<HTMLAnchorElement>("a.errata-hunk")];
    first?.click();
    expect(view.state.selection.main.from).toBe(0);
    expect(view.state.selection.main.to).toBe("one\nTWO\nthree".length);
    removal?.click();
    const sixth = TEXT.indexOf("six");
    expect([view.state.selection.main.from, view.state.selection.main.to]).toEqual([sixth, sixth]);
    pane(root).querySelector<HTMLElement>(".diff-add")?.click();
    expect(view.state.selection.main.to).toBe("one\nTWO\nthree".length);
  });

  it("stops jumping once the text is edited", async () => {
    const root = await openRfc(withErrata({}));
    const view = mainView(root);
    view.dispatch({ changes: { from: 0, insert: "zero\n" } });
    view.dispatch({ selection: { anchor: 2 } });
    pane(root).querySelector<HTMLAnchorElement>("a.errata-hunk")?.click();
    expect(view.state.selection.main.from).toBe(2);
    expect(pane(root).textContent).toContain("The text was changed after it was opened");
    expect(pane(root).querySelector("a.errata-hunk")).toBeNull();
  });

  it("says when no errata were applied, and when an RFC's errata failed", async () => {
    const root = await openRfc(withErrata({ errata: [] }));
    expect(pane(root).textContent).toBe("No errata applied.");
    expect(badge(root).classList.contains("d-none")).toBe(true);

    const failed = await openRfc(withErrata({ errata: [], errata_failure: { errata_id: 9, reason: "hunk 1: not in the text" } }));
    expect(pane(failed).textContent).toContain("could not be applied (erratum 9: hunk 1: not in the text)");
    expect(pane(failed).textContent).toContain("No errata applied.");
  });

  it("shows no errata for pasted text", async () => {
    answerWith(RESPONSE);
    const root = mount();
    createChecker(root);
    checkButton(root)?.click();
    await vi.waitFor(() => expect(root.textContent).toContain("differs from its definition"));
    expect(pane(root).textContent).toBe("No errata applied.");
  });
});

describe("the two views of an RFC from RFCXML", () => {
  const TEXT = "1.  Introduction\n\n   Thing ::= INTEGER\n";
  const XML = '<rfc>\n<section>\n<sourcecode>\nThing ::= INTEGER\n</sourcecode>\n</section>\n</rfc>\n';
  const SOURCE = "https://www.rfc-editor.org/rfc/rfc1000.xml";
  const W202 = {
    code: "W202",
    severity: "warning" as const,
    message: "text looks like ASN.1 but is not marked as `sourcecode` of type `asn.1`",
    primary: { file: 0, span: { start: 18, end: 54 }, line_col: [3, 1] as [number, number], section: "1", text: null },
    secondary: [],
    notes: [],
    help: null,
  };
  const erratum = { id: 7, by: "human", note: "Thing is an INTEGER" };
  const VIEWS: Record<"text" | "xml", CheckResponse> = {
    text: {
      ...RESPONSE,
      diagnostics: [],
      document: {
        name: "rfc1000",
        format: "text",
        content: TEXT,
        xml_source: SOURCE,
        other_format: "xml",
        markup_findings: { errors: 0, warnings: 2, infos: 0 },
        errata: [{ ...erratum, patch: "--- a/rfc1000.txt\n+++ b/rfc1000.txt\n@@ -3 +3 @@\n-   Thing ::= BOOLEAN\n+   Thing ::= INTEGER\n", hunks: [{ start_line: 3, end_line: 3 }] }],
      },
    },
    xml: {
      ...RESPONSE,
      diagnostics: [W202, { ...W202, primary: { ...W202.primary, span: { start: 60, end: 61 } } }],
      document: {
        name: "rfc1000.xml",
        format: "xml",
        content: XML,
        xml_source: SOURCE,
        other_format: "text",
        errata: [{ ...erratum, patch: "--- a/rfc1000.xml\n+++ b/rfc1000.xml\n@@ -4 +4 @@\n-Thing ::= BOOLEAN\n+Thing ::= INTEGER\n", hunks: [{ start_line: 4, end_line: 4 }] }],
      },
    },
  };

  function serveViews() {
    return useBackend((_method, args) => VIEWS[(args[0] as { format?: "xml" }).format ?? "text"]);
  }

  const titles = (root: HTMLElement) => [...root.querySelectorAll(".doc-tab-title")].map((t) => t.textContent);
  const activeTitle = (root: HTMLElement) => root.querySelector(".doc-tab.active .doc-tab-title")?.textContent;
  const viewLink = (root: HTMLElement) => root.querySelector<HTMLAnchorElement>(".view-links a:not([target])");
  const errataPane = (root: HTMLElement) => root.querySelector("#pane-errata") as HTMLElement;
  const editorText = (root: HTMLElement) => mainView(root).state.doc.toString();

  async function openText(): Promise<{ root: HTMLElement; call: ReturnType<typeof serveViews> }> {
    const call = serveViews();
    const root = mount();
    createChecker(root).show({ view: "checker", rfc: 1000 });
    await vi.waitFor(() => expect(editorText(root)).toBe(TEXT));
    return { root, call };
  }

  it("links the other view and the published XML beside the status line, outside the panes", async () => {
    const { root } = await openText();
    const link = viewLink(root);
    expect(link?.textContent).toBe("RFCXML");
    expect(link?.getAttribute("href")).toBe("#rfc1000.xml");
    expect(link?.closest(".toolbar")).not.toBeNull();
    const published = root.querySelector<HTMLAnchorElement>(".view-links a[target]");
    expect(published?.href).toBe(SOURCE);
    expect(published?.target).toBe("_blank");
    expect(published?.rel).toBe("noopener");
    // The errata tab no longer carries it.
    expect(errataPane(root).textContent).not.toContain("RFCXML");
  });

  it("opens the RFCXML in a tab of its own, and brings an open view's tab forward instead of opening it again", async () => {
    const { root, call } = await openText();
    viewLink(root)?.click();
    await vi.waitFor(() => expect(editorText(root)).toBe(XML));
    expect(titles(root)).toEqual(["rfc1000", "rfc1000.xml"]);
    expect(activeTitle(root)).toBe("rfc1000.xml");
    expect(call.mock.calls.map((c) => c[1][0])).toEqual([
      expect.not.objectContaining({ format: expect.anything() }),
      expect.objectContaining({ rfc: 1000, format: "xml" }),
    ]);
    expect(viewLink(root)?.textContent).toBe("Text");
    expect(viewLink(root)?.getAttribute("href")).toBe("#rfc1000");

    viewLink(root)?.click();
    expect(activeTitle(root)).toBe("rfc1000");
    expect(editorText(root)).toBe(TEXT);
    viewLink(root)?.click();
    expect(activeTitle(root)).toBe("rfc1000.xml");
    expect(titles(root)).toEqual(["rfc1000", "rfc1000.xml"]);
    expect(call).toHaveBeenCalledTimes(2);
  });

  it("stands for the findings about the markup with one line in Diagnostics, which opens the RFCXML", async () => {
    const { root } = await openText();
    const pane = root.querySelector("#pane-diagnostics") as HTMLElement;
    const line = pane.querySelector<HTMLAnchorElement>(".markup-findings a");
    expect(line?.textContent).toBe("2 findings about the RFCXML markup");
    expect(line?.getAttribute("href")).toBe("#rfc1000.xml");
    expect(pane.textContent).toContain("No other findings.");
    // They count with the text view's own findings, so the view opened by default does not hide them.
    expect(root.querySelector("#pane-diagnostics-tab .badge")?.textContent).toBe("2");
    expect(root.textContent).toContain("0 errors, 2 warnings (2 about the RFCXML markup)");
    line?.click();
    await vi.waitFor(() => expect(editorText(root)).toBe(XML));
    // The XML view shows them on its elements, and has no such line.
    expect([...pane.querySelectorAll(".diag .badge")].map((b) => b.textContent)).toEqual(["W202", "W202"]);
    expect(pane.querySelector(".markup-findings")).toBeNull();
  });

  it("shows each view's own errata patches, and jumps to that view's lines", async () => {
    const { root } = await openText();
    const diff = () => [...errataPane(root).querySelectorAll(".diff-add")].map((e) => e.textContent);
    expect(diff()).toEqual(["+   Thing ::= INTEGER"]);
    errataPane(root).querySelector<HTMLAnchorElement>("a.errata-hunk")?.click();
    expect(mainView(root).state.selection.main.from).toBe(TEXT.indexOf("   Thing"));

    viewLink(root)?.click();
    await vi.waitFor(() => expect(editorText(root)).toBe(XML));
    expect(diff()).toEqual(["+Thing ::= INTEGER"]);
    expect(errataPane(root).querySelector(".errata-hunk")?.textContent).toBe("line 4");
    errataPane(root).querySelector<HTMLAnchorElement>("a.errata-hunk")?.click();
    expect(mainView(root).state.selection.main.from).toBe(XML.indexOf("Thing ::="));
  });

  it("says when the edited text is read without the RFCXML marking", async () => {
    const { root } = await openText();
    const status = () => root.querySelector('[role="status"]')?.textContent ?? "";
    expect(status()).not.toContain("edited text");
    // The edited text is checked as pasted text: the answer names no other view.
    answerWith({ ...RESPONSE, diagnostics: [], document: { name: "rfc1000", format: "text" } });
    mainView(root).dispatch({ changes: { from: 0, insert: "zero\n" } });
    checkButton(root)?.click();
    await vi.waitFor(() => expect(status()).toContain("ASN.1 found by reading the edited text, not from the RFCXML marking"));
  });
});

describe("the two views of a draft from RFCXML", () => {
  const DRAFT = "draft-ietf-x-thing-03";
  const TEXT = "Internet-Draft    Thing    October 2026\n\n   Thing ::= INTEGER\n";
  const XML = '<rfc>\n<section>\n<sourcecode>\nThing ::= INTEGER\n</sourcecode>\n</section>\n</rfc>\n';
  const SOURCE = `https://www.ietf.org/archive/id/${DRAFT}.xml`;
  const VIEWS: Record<"text" | "xml", CheckResponse> = {
    text: {
      ...RESPONSE,
      diagnostics: [],
      document: {
        name: DRAFT,
        format: "text",
        content: TEXT,
        xml_source: SOURCE,
        other_format: "xml",
        markup_findings: { errors: 0, warnings: 1, infos: 0 },
        errata: [],
      },
    },
    xml: {
      ...RESPONSE,
      diagnostics: [],
      document: { name: `${DRAFT}.xml`, format: "xml", content: XML, xml_source: SOURCE, other_format: "text", errata: [] },
    },
  };

  const editorText = (root: HTMLElement) => mainView(root).state.doc.toString();
  const viewLink = (root: HTMLElement) => root.querySelector<HTMLAnchorElement>(".view-links a:not([target])");
  const activeTitle = (root: HTMLElement) => root.querySelector(".doc-tab.active .doc-tab-title")?.textContent;

  function serveViews() {
    return useBackend((_method, args) => VIEWS[(args[0] as { format?: "xml" }).format ?? "text"]);
  }

  it("opens the text, links the RFCXML view and the archive's XML, and opens the RFCXML by its route", async () => {
    const call = serveViews();
    const root = mount();
    createChecker(root).show({ view: "checker", draft: DRAFT });
    await vi.waitFor(() => expect(editorText(root)).toBe(TEXT));
    expect(call.mock.calls[0]?.[1][0]).toEqual(expect.objectContaining({ draft: DRAFT }));
    expect(call.mock.calls[0]?.[1][0]).not.toHaveProperty("format");
    const link = viewLink(root);
    expect(link?.textContent).toBe("RFCXML");
    expect(link?.getAttribute("href")).toBe(`#${DRAFT}.xml`);
    expect(link?.title).toBe("Open the RFCXML of this draft, with its findings about the markup");
    const published = root.querySelector<HTMLAnchorElement>(".view-links a[target]");
    expect(published?.href).toBe(SOURCE);
    expect(published?.getAttribute("aria-label")).toBe("Published RFCXML at ietf.org");
    expect(root.querySelector(".markup-findings a")?.getAttribute("href")).toBe(`#${DRAFT}.xml`);

    link?.click();
    await vi.waitFor(() => expect(editorText(root)).toBe(XML));
    expect(call.mock.calls[1]?.[1][0]).toEqual(expect.objectContaining({ draft: DRAFT, format: "xml" }));
    expect(activeTitle(root)).toBe(`${DRAFT}.xml`);
    expect(viewLink(root)?.textContent).toBe("Text");
    expect(viewLink(root)?.getAttribute("href")).toBe(`#${DRAFT}`);
  });

  it("opens the RFCXML of a draft typed with its .xml file name", async () => {
    const call = serveViews();
    const root = mount();
    createChecker(root);
    const input = root.querySelector<HTMLInputElement>('input[type="text"]') as HTMLInputElement;
    input.value = `${DRAFT}.xml`;
    root.querySelector("form")?.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    await vi.waitFor(() => expect(editorText(root)).toBe(XML));
    expect(call.mock.calls[0]?.[1][0]).toEqual(expect.objectContaining({ draft: DRAFT, format: "xml" }));
    expect(activeTitle(root)).toBe(`${DRAFT}.xml`);
  });
});
