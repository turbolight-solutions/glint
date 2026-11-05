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
