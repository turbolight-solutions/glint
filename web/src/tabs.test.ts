// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { createTabStrip, nextActive } from "./tabs";

describe("nextActive", () => {
  const ids = [1, 2, 3, 4];

  it("keeps the active tab when it survives", () => {
    expect(nextActive(ids, new Set([1, 4]), 2)).toBe(2);
  });

  it("moves to the right neighbour when the active tab closes", () => {
    expect(nextActive(ids, new Set([2]), 2)).toBe(3);
  });

  it("falls back to the left when nothing is to the right", () => {
    expect(nextActive(ids, new Set([4]), 4)).toBe(3);
    expect(nextActive(ids, new Set([3, 4]), 4)).toBe(2);
  });

  it("skips tabs that close with it", () => {
    expect(nextActive(ids, new Set([1, 2, 3]), 2)).toBe(4);
  });

  it("returns null when everything closes", () => {
    expect(nextActive(ids, new Set(ids), 2)).toBeNull();
  });
});

function setup() {
  const calls: string[] = [];
  const strip = createTabStrip({
    select: (id) => calls.push(`select ${id}`),
    close: (id) => calls.push(`close ${id}`),
    closeOthers: (id) => calls.push(`others ${id}`),
    closeAll: () => calls.push("all"),
    add: () => calls.push("add"),
  });
  document.body.replaceChildren(strip.element);
  const render = (count: number, active: number) =>
    strip.render(
      Array.from({ length: count }, (_, i) => ({ id: i + 1, title: `doc ${i + 1}`, busy: false })),
      active,
    );
  const label = (id: number) => document.querySelector<HTMLElement>(`[data-tab-id="${id}"] .doc-tab-label`) as HTMLElement;
  const menuItem = (text: string) =>
    [...document.querySelectorAll<HTMLButtonElement>(".tab-menu button")].find((b) => b.textContent === text);
  return { calls, render, label, menuItem };
}

afterEach(() => {
  document.body.replaceChildren();
  vi.useRealTimers();
});

describe("tab strip", () => {
  it("selects on click and closes with the close button or a middle click", () => {
    const { calls, render, label } = setup();
    render(3, 1);
    label(2).click();
    document.querySelector<HTMLElement>('[data-tab-id="3"] .doc-tab-close')?.click();
    label(1).dispatchEvent(new MouseEvent("auxclick", { button: 1, bubbles: true, cancelable: true }));
    expect(calls).toEqual(["select 2", "close 3", "close 1"]);
  });

  it("marks only the active tab as selected", () => {
    const { render } = setup();
    render(3, 2);
    const selected = [...document.querySelectorAll('[role="tab"]')].map((t) => t.getAttribute("aria-selected"));
    expect(selected).toEqual(["false", "true", "false"]);
  });

  it("offers close, close others and close all on right click", () => {
    const { calls, render, label, menuItem } = setup();
    render(3, 1);
    label(2).dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 10, clientY: 10 }));
    expect(menuItem("Close")).toBeDefined();
    menuItem("Close others")?.click();
    expect(document.querySelector(".tab-menu")).toBeNull();
    label(3).dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 10, clientY: 10 }));
    menuItem("Close all")?.click();
    label(1).dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 10, clientY: 10 }));
    menuItem("Close")?.click();
    expect(calls).toEqual(["others 2", "all", "close 1"]);
  });

  it("disables close others when there is only one tab", () => {
    const { render, label, menuItem } = setup();
    render(1, 1);
    label(1).dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 1, clientY: 1 }));
    expect(menuItem("Close others")?.disabled).toBe(true);
  });

  it("dismisses the menu on Escape and on an outside press", () => {
    const { render, label } = setup();
    render(2, 1);
    const open = () =>
      label(1).dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 1, clientY: 1 }));
    open();
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    expect(document.querySelector(".tab-menu")).toBeNull();
    open();
    document.body.dispatchEvent(new Event("pointerdown", { bubbles: true }));
    expect(document.querySelector(".tab-menu")).toBeNull();
  });

  it("opens the menu on a touch long press, and not on a quick tap", () => {
    vi.useFakeTimers();
    const { calls, render, label } = setup();
    render(2, 1);
    const touch = (type: string, x = 5) =>
      label(2).dispatchEvent(
        Object.assign(new MouseEvent(type, { bubbles: true, clientX: x, clientY: 5 }), { pointerType: "touch" }),
      );
    touch("pointerdown");
    vi.advanceTimersByTime(100);
    touch("pointerup");
    vi.advanceTimersByTime(1000);
    expect(document.querySelector(".tab-menu")).toBeNull();

    touch("pointerdown");
    vi.advanceTimersByTime(600);
    expect(document.querySelector(".tab-menu")).not.toBeNull();
    // The click that follows lifting the finger must not switch tabs.
    label(2).click();
    expect(calls).toEqual([]);
  });

  it("moves between tabs with the arrow keys", () => {
    const { calls, render, label } = setup();
    render(3, 1);
    label(1).dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true, cancelable: true }));
    label(1).dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowLeft", bubbles: true, cancelable: true }));
    expect(calls).toEqual(["select 2", "select 3"]);
  });
});
