// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ModuleListItem, ModulePage } from "./api";
import { createCorpusBrowser } from "./corpus";

const TOTAL = 120;

function item(index: number): ModuleListItem {
  return { name: `Module${index}`, oid: null, source: "rfc1000", rfc: 1000, section: null, status: "clean", obsoleted_by: [] };
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

  it("explains a server that still answers with a bare array", async () => {
    vi.stubGlobal("fetch", vi.fn(async (url: string) => (url.startsWith("/api/v1/corpus") ? respond(url) : new Response("[]", { status: 200 }))));
    document.body.innerHTML = '<section id="root"></section>';
    const root = document.getElementById("root") as HTMLElement;
    createCorpusBrowser(root).show("");
    await settle();
    expect(root.textContent).toContain("Restart glint-server");
  });
});
