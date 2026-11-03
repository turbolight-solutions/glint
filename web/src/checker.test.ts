// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import type { CheckResponse } from "./api";
import { createChecker } from "./checker";

const RESPONSE: CheckResponse = {
  schema_version: 1,
  corpus_version: "test",
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

afterEach(() => vi.unstubAllGlobals());

describe("checker view", () => {
  it("renders diagnostics, modules and import links after a check", async () => {
    const calls: string[] = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string, init?: RequestInit) => {
        calls.push(`${url} ${init?.body ?? ""}`);
        return new Response(JSON.stringify(RESPONSE), { status: 200 });
      }),
    );
    const root = mount();
    createChecker(root);
    checkButton(root)?.click();
    await vi.waitFor(() => expect(root.textContent).toContain("differs from its definition"));
    expect(root.textContent).toContain("1 error");
    expect(root.querySelector('a[href="#module/rfc1000/Base"]')).not.toBeNull();
    expect(root.querySelector('a[href^="#module/rfc1000/Base?from=10&to=14"]')).not.toBeNull();
    expect(calls[0]).toContain("/api/v1/check");
  });

  it("shows a server error instead of throwing", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(
        async () =>
          new Response(JSON.stringify({ error: { code: "check_timeout", message: "slow" } }), { status: 504 }),
      ),
    );
    const root = mount();
    createChecker(root);
    checkButton(root)?.click();
    await vi.waitFor(() => expect(root.textContent).toContain("took too long"));
  });

  it("fetches a draft named in the route and fills the editor", async () => {
    const withContent = {
      ...RESPONSE,
      document: { name: "draft-a-b-01", format: "bare" as const, content: "Thing ::= INTEGER" },
    };
    const fetchMock = vi.fn(async () => new Response(JSON.stringify(withContent), { status: 200 }));
    vi.stubGlobal("fetch", fetchMock);
    const root = mount();
    const checker = createChecker(root);
    checker.show({ view: "checker", draft: "draft-a-b-01" });
    await vi.waitFor(() => expect(root.textContent).toContain("Thing ::= INTEGER"));
    const [url, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toBe("/api/v1/check/draft");
    expect(JSON.parse(String(init.body))).toMatchObject({ draft: "draft-a-b-01" });
  });
});
