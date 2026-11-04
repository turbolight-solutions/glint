// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ModuleListItem, ModulePage } from "./api";
import { createCorpusBrowser, createOidPage } from "./corpus";

const TOTAL = 120;

function item(index: number): ModuleListItem {
  return { name: `Module${index}`, oid: null, body: "rfc", document: "rfc1000", edition: null, url: null, section: null, status: "clean", obsoleted_by: [] };
}

function respond(url: string): Response {
  if (url.startsWith("/api/v1/corpus")) {
    return new Response(JSON.stringify({ version: "t", modules: TOTAL, oid_assignments: 0, by_status: {}, notices: [] }), { status: 200 });
  }
  const params = new URL(url, "http://localhost").searchParams;
  const offset = Number(params.get("offset"));
  const limit = Number(params.get("limit"));
  const count = Math.max(0, Math.min(limit, TOTAL - offset));
  const page: ModulePage = { total: TOTAL, offset, limit, modules: Array.from({ length: count }, (_, i) => item(offset + i)) };
  return new Response(JSON.stringify(page), { status: 200 });
}

function button(root: HTMLElement, label: string): HTMLButtonElement {
  const found = [...root.querySelectorAll("button")].find((b) => b.textContent === label);
  if (!found) throw new Error(`no ${label} button`);
  return found;
}

async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 20));
}

afterEach(() => vi.unstubAllGlobals());

describe("corpus browser paging", () => {
  it("steps through the list with Next and Previous", async () => {
    const fetchMock = vi.fn(async (url: string) => respond(url));
    vi.stubGlobal("fetch", fetchMock);
    document.body.innerHTML = '<section id="root"></section>';
    const root = document.getElementById("root") as HTMLElement;
    vi.stubGlobal("scrollTo", vi.fn());
    window.scrollTo = vi.fn() as unknown as typeof window.scrollTo;
    createCorpusBrowser(root).show("");
    await settle();

    expect(root.textContent).toContain("1–50 of 120");
    expect(root.textContent).toContain("Module0");
    expect(button(root, "Previous").disabled).toBe(true);

    button(root, "Next").click();
    await settle();
    expect(root.textContent).toContain("51–100 of 120");
    expect(root.textContent).toContain("Module50");
    expect(root.textContent).not.toContain("Module0 ");

    button(root, "Next").click();
    await settle();
    expect(root.textContent).toContain("101–120 of 120");
    expect(button(root, "Next").disabled).toBe(true);

    button(root, "Previous").click();
    await settle();
    expect(root.textContent).toContain("51–100 of 120");
  });

  it("links an RFC module to the editor and a module of another body to its own document", async () => {
    const itu = { ...item(1), name: "Itu1", body: "itu-t", document: "X.509", edition: "10/2012", url: null };
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string) =>
        url.startsWith("/api/v1/corpus")
          ? respond(url)
          : new Response(JSON.stringify({ total: 2, offset: 0, limit: 50, modules: [item(0), itu] }), { status: 200 }),
      ),
    );
    document.body.innerHTML = '<section id="root"></section>';
    const root = document.getElementById("root") as HTMLElement;
    createCorpusBrowser(root).show("");
    await settle();
    expect(root.querySelector('a[href="#rfc1000?module=Module0"]')).not.toBeNull();
    expect(root.querySelector('a[href="#X.509?module=Itu1"]')).not.toBeNull();
  });

  it("links each of two same-named modules in one RFC to its own OID", async () => {
    const twin = (oid: string): ModuleListItem => ({ ...item(0), name: "ERS", document: "rfc4998", oid });
    const itu = { ...item(1), name: "Itu1", body: "itu-t", document: "X.509", edition: "10/2012", url: null, oid: "2.5.1.1.7" };
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string) =>
        url.startsWith("/api/v1/corpus")
          ? respond(url)
          : new Response(JSON.stringify({ total: 3, offset: 0, limit: 50, modules: [twin("1.3.6.1.5.5.11.0.1.1"), twin("1.3.6.1.5.5.11.0.2.1"), itu] }), { status: 200 }),
      ),
    );
    document.body.innerHTML = '<section id="root"></section>';
    const root = document.getElementById("root") as HTMLElement;
    createCorpusBrowser(root).show("");
    await settle();
    expect(root.querySelector('a[href="#rfc4998?module=ERS&oid=1.3.6.1.5.5.11.0.1.1"]')).not.toBeNull();
    expect(root.querySelector('a[href="#rfc4998?module=ERS&oid=1.3.6.1.5.5.11.0.2.1"]')).not.toBeNull();
    expect(root.querySelector('a[href="#X.509?module=Itu1&oid=2.5.1.1.7"]')).not.toBeNull();
  });

  it("explains a server that still answers with a bare array", async () => {
    vi.stubGlobal("fetch", vi.fn(async (url: string) => (url.startsWith("/api/v1/corpus") ? respond(url) : new Response("[]", { status: 200 }))));
    document.body.innerHTML = '<section id="root"></section>';
    const root = document.getElementById("root") as HTMLElement;
    createCorpusBrowser(root).show("");
    await settle();
    expect(root.textContent).toContain("Restart glint-server");
  });
});

describe("corpus browser sorting", () => {
  it("sorts the whole list by severity across pages and pages through the sorted list", async () => {
    // Module119 is the only module with errors, and it would be on the third page in the server's order.
    const fetchMock = vi.fn(async (url: string) => {
      if (url.startsWith("/api/v1/corpus")) return respond(url);
      const modules = Array.from({ length: TOTAL }, (_, i) => ({ ...item(i), status: i === TOTAL - 1 ? ("errors" as const) : ("clean" as const) }));
      const params = new URL(url, "http://localhost").searchParams;
      const offset = Number(params.get("offset"));
      const limit = Number(params.get("limit"));
      return new Response(JSON.stringify({ total: TOTAL, offset, limit, modules: modules.slice(offset, offset + limit) }), { status: 200 });
    });
    vi.stubGlobal("fetch", fetchMock);
    document.body.innerHTML = '<section id="root"></section>';
    const root = document.getElementById("root") as HTMLElement;
    window.scrollTo = vi.fn() as unknown as typeof window.scrollTo;
    window.location.hash = "";
    const browser = createCorpusBrowser(root);
    browser.show("");
    await settle();
    expect(root.textContent).not.toContain("Module119");

    button(root, "Status").click();
    expect(root.querySelector("th[aria-sort]")?.getAttribute("aria-sort")).toBe("descending");
    expect(root.querySelector("tbody tr")?.textContent).toContain("Module119");
    expect(root.textContent).toContain("1–50 of 120");

    button(root, "Next").click();
    expect(root.textContent).toContain("51–100 of 120");

    button(root, "Status ▼").click();
    expect(root.querySelector("th[aria-sort]")?.getAttribute("aria-sort")).toBe("ascending");
    expect(root.textContent).toContain("1–50 of 120");
    expect(root.textContent).not.toContain("Module119");

    button(root, "Status ▲").click();
    expect(root.querySelector("th[aria-sort]")).toBeNull();
    expect(window.location.hash).toBe("#corpus");

    button(root, "OID").click();
    expect(window.location.hash).toBe("#corpus?sort=oid");
    button(root, "OID ▲").click();
    expect(window.location.hash).toBe("#corpus?sort=-oid");

    // Opening a link with a sort in it shows that sort, and moving between sorts does not fetch again.
    browser.show("", { key: "status", direction: "desc" });
    expect(root.querySelector("tbody tr")?.textContent).toContain("Module119");
    browser.show("", null);
    expect(root.querySelector("th[aria-sort]")).toBeNull();

    // The list was fetched once, in one request, and sorting and paging never went back to the server.
    const moduleRequests = fetchMock.mock.calls.filter(([url]) => String(url).startsWith("/api/v1/modules"));
    expect(moduleRequests).toHaveLength(1);
    expect(String(moduleRequests[0]?.[0])).toContain("limit=500");
  });

  it("keeps fetching until it has every module when the server answers with less than asked", async () => {
    const fetchMock = vi.fn(async (url: string) => {
      if (url.startsWith("/api/v1/corpus")) return respond(url);
      const params = new URL(url, "http://localhost").searchParams;
      const offset = Number(params.get("offset"));
      const count = Math.min(70, TOTAL - offset);
      return new Response(JSON.stringify({ total: TOTAL, offset, limit: 70, modules: Array.from({ length: count }, (_, i) => item(offset + i)) }), { status: 200 });
    });
    vi.stubGlobal("fetch", fetchMock);
    document.body.innerHTML = '<section id="root"></section>';
    const root = document.getElementById("root") as HTMLElement;
    createCorpusBrowser(root).show("");
    await settle();
    expect(root.textContent).toContain("1–50 of 120");
  });
});

describe("OID lookup page", () => {
  function setup(): { root: HTMLElement; input: HTMLInputElement; fetchMock: ReturnType<typeof vi.fn> } {
    const fetchMock = vi.fn(
      async (url: string) =>
        new Response(JSON.stringify({ oid: decodeURIComponent(url.split("/").pop() ?? ""), parent: null, module: null, assignments: [], children: [], truncated: false }), {
          status: 200,
        }),
    );
    vi.stubGlobal("fetch", fetchMock);
    document.body.innerHTML = '<section id="root"></section>';
    const root = document.getElementById("root") as HTMLElement;
    window.location.hash = "";
    createOidPage(root).show();
    return { root, input: root.querySelector("input") as HTMLInputElement, fetchMock };
  }

  function submit(root: HTMLElement): void {
    root.querySelector("form")?.dispatchEvent(new Event("submit", { cancelable: true }));
  }

  it("looks up the example OID when the box is empty", () => {
    const { root } = setup();
    submit(root);
    expect(window.location.hash).toBe("#oid/1.3.6.1.5.5.7");
  });

  it("reduces names with numbers in parentheses to dotted digits", () => {
    const { root, input } = setup();
    input.value = "iso(1)\n  identified-organization(3) dod(6) internet(1)";
    submit(root);
    expect(window.location.hash).toBe("#oid/1.3.6.1");
  });

  it("warns about a wrong arc name", () => {
    const { root, input } = setup();
    input.value = "iso(1) identified-organization(3) dod(6) internet(1) security(5) mechanisms(5) pkcs(7)";
    submit(root);
    expect(root.textContent).toContain("1.3.6.1.5.5.7 is pkix, not pkcs.");
  });

  it("says so when the text is not an OID", () => {
    const { root, input } = setup();
    input.value = "not an oid";
    submit(root);
    expect(root.textContent).toContain("That is not an OID");
  });

  it("looks up again when the OID is already in the address bar", async () => {
    const { root, fetchMock } = setup();
    window.location.hash = "#oid/1.3.6.1.5.5.7";
    submit(root);
    await settle();
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });
});
