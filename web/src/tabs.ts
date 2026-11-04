import { h } from "./dom";

export interface TabInfo {
  id: number;
  title: string;
  /** A fetch is in flight and the tab cannot be reused for another document yet. */
  busy: boolean;
}

export interface TabHandlers {
  select(id: number): void;
  close(id: number): void;
  closeOthers(id: number): void;
  closeAll(): void;
  add(): void;
}

export interface TabStrip {
  element: HTMLElement;
  render(tabs: TabInfo[], activeId: number): void;
}

const LONG_PRESS_MS = 500;
const LONG_PRESS_SLOP_PX = 10;

/**
 * The tab that should be active once `closing` is gone: the active one if it survives,
 * otherwise its right neighbour, otherwise the nearest one on the left. `null` when nothing
 * is left.
 */
export function nextActive(ids: readonly number[], closing: ReadonlySet<number>, activeId: number): number | null {
  if (!closing.has(activeId) && ids.includes(activeId)) return activeId;
  const at = ids.indexOf(activeId);
  const after = ids.slice(at + 1).find((id) => !closing.has(id));
  if (after !== undefined) return after;
  const before = ids.slice(0, Math.max(at, 0)).reverse().find((id) => !closing.has(id));
  return before ?? null;
}

export function createTabStrip(handlers: TabHandlers): TabStrip {
  let tabs: TabInfo[] = [];
  let activeId = -1;
  let focusAfterRender: number | null = null;
  let closeMenu: (() => void) | null = null;
  let suppressClickUntil = 0;

  const list = h("ul", { class: "doc-tabs", role: "tablist", "aria-label": "Open documents" });
  const add = h(
    "button",
    { class: "btn btn-sm doc-tab-add", type: "button", "aria-label": "New tab", title: "New tab", onclick: () => handlers.add() },
    "+",
  );
  const element = h("div", { class: "doc-tab-strip" }, list, add);

  const idOf = (target: EventTarget | null): number | null => {
    const found = target instanceof Element ? target.closest<HTMLElement>("[data-tab-id]") : null;
    return found ? Number(found.dataset.tabId) : null;
  };
  const labelOf = (id: number): HTMLElement | null => list.querySelector<HTMLElement>(`[data-tab-id="${id}"] .doc-tab-label`);

  function openMenu(id: number, x: number, y: number): void {
    closeMenu?.();
    const trigger = labelOf(id);
    const item = (label: string, action: () => void, disabled = false) =>
      h(
        "button",
        {
          class: "dropdown-item",
          type: "button",
          role: "menuitem",
          disabled,
          onclick: () => {
            closeMenu?.();
            action();
          },
        },
        label,
      );
    const menu = h(
      "div",
      { class: "dropdown-menu show tab-menu", role: "menu", "aria-label": "Tab actions" },
      item("Close", () => handlers.close(id)),
      item("Close others", () => handlers.closeOthers(id), tabs.length < 2),
      item("Close all", () => handlers.closeAll()),
    );
    document.body.appendChild(menu);
    const box = menu.getBoundingClientRect();
    menu.style.left = `${Math.max(0, Math.min(x, window.innerWidth - box.width))}px`;
    menu.style.top = `${Math.max(0, Math.min(y, window.innerHeight - box.height))}px`;

    const items = [...menu.querySelectorAll<HTMLButtonElement>("button:not(:disabled)")];
    const onPointerDown = (event: Event) => {
      if (!(event.target instanceof Node && menu.contains(event.target))) closeMenu?.();
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        const returnTo = trigger;
        closeMenu?.();
        returnTo?.focus();
      } else if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        const at = items.indexOf(document.activeElement as HTMLButtonElement);
        const step = event.key === "ArrowDown" ? 1 : -1;
        items[(at + step + items.length) % items.length]?.focus();
      } else if (event.key === "Tab") {
        closeMenu?.();
      }
    };
    const dismiss = () => closeMenu?.();
    document.addEventListener("pointerdown", onPointerDown, true);
    document.addEventListener("keydown", onKey, true);
    document.addEventListener("scroll", dismiss, true);
    window.addEventListener("resize", dismiss);
    window.addEventListener("blur", dismiss);
    const self = () => {
      document.removeEventListener("pointerdown", onPointerDown, true);
      document.removeEventListener("keydown", onKey, true);
      document.removeEventListener("scroll", dismiss, true);
      window.removeEventListener("resize", dismiss);
      window.removeEventListener("blur", dismiss);
      menu.remove();
      if (closeMenu === self) closeMenu = null;
    };
    closeMenu = self;
    items[0]?.focus();
  }

  list.addEventListener("click", (event) => {
    const id = idOf(event.target);
    if (id === null) return;
    if (performance.now() < suppressClickUntil) return;
    if ((event.target as Element).closest(".doc-tab-close")) handlers.close(id);
    else handlers.select(id);
  });

  // Middle click closes, as in a browser.
  list.addEventListener("mousedown", (event) => {
    if (event.button === 1 && idOf(event.target) !== null) event.preventDefault();
  });
  list.addEventListener("auxclick", (event) => {
    const id = idOf(event.target);
    if (event.button !== 1 || id === null) return;
    event.preventDefault();
    handlers.close(id);
  });

  // Right click, the menu key, and (on Android) long press all arrive as `contextmenu`.
  list.addEventListener("contextmenu", (event) => {
    const id = idOf(event.target);
    if (id === null) return;
    event.preventDefault();
    let { clientX: x, clientY: y } = event;
    if (x === 0 && y === 0) {
      // Keyboard invocation reports no pointer position; hang the menu off the tab instead.
      const box = labelOf(id)?.getBoundingClientRect();
      if (box) [x, y] = [box.left, box.bottom];
    }
    openMenu(id, x, y);
  });

  // iOS never sends `contextmenu` for a long press, so do it by hand for touch input.
  let press: { timer: ReturnType<typeof setTimeout>; x: number; y: number } | null = null;
  const cancelPress = () => {
    if (press) clearTimeout(press.timer);
    press = null;
  };
  list.addEventListener("pointerdown", (event) => {
    const id = idOf(event.target);
    if (event.pointerType !== "touch" || id === null) return;
    cancelPress();
    const { clientX: x, clientY: y } = event;
    press = {
      x,
      y,
      timer: setTimeout(() => {
        press = null;
        // The finger is still down; the click that follows its release must not switch tabs.
        suppressClickUntil = performance.now() + 1000;
        openMenu(id, x, y);
      }, LONG_PRESS_MS),
    };
  });
  list.addEventListener("pointermove", (event) => {
    if (press && Math.hypot(event.clientX - press.x, event.clientY - press.y) > LONG_PRESS_SLOP_PX) cancelPress();
  });
  list.addEventListener("pointerup", cancelPress);
  list.addEventListener("pointercancel", cancelPress);

  list.addEventListener("keydown", (event) => {
    const id = idOf(event.target);
    if (id === null) return;
    const at = tabs.findIndex((t) => t.id === id);
    let to: number | undefined;
    switch (event.key) {
      case "ArrowRight":
        to = (at + 1) % tabs.length;
        break;
      case "ArrowLeft":
        to = (at - 1 + tabs.length) % tabs.length;
        break;
      case "Home":
        to = 0;
        break;
      case "End":
        to = tabs.length - 1;
        break;
      default:
        return;
    }
    event.preventDefault();
    const next = tabs[to];
    if (!next) return;
    focusAfterRender = next.id;
    handlers.select(next.id);
  });

  return {
    element,
    render(next, active) {
      tabs = next;
      activeId = active;
      const refocus = focusAfterRender ?? (list.contains(document.activeElement) ? idOf(document.activeElement) : null);
      focusAfterRender = null;
      list.replaceChildren(
        ...tabs.map((tab) => {
          const selected = tab.id === activeId;
          return h(
            "li",
            { class: `doc-tab${selected ? " active" : ""}`, "data-tab-id": tab.id, role: "presentation" },
            h(
              "button",
              {
                class: "doc-tab-label",
                type: "button",
                role: "tab",
                title: tab.title,
                "aria-selected": String(selected),
                tabindex: selected ? 0 : -1,
              },
              tab.busy ? h("span", { class: "spinner-border spinner-border-sm me-1", "aria-hidden": "true" }) : null,
              h("span", { class: "doc-tab-title" }, tab.title),
            ),
            h(
              "button",
              { class: "doc-tab-close", type: "button", tabindex: -1, "aria-label": `Close ${tab.title}`, title: "Close" },
              "×",
            ),
          );
        }),
      );
      if (refocus !== null) labelOf(refocus)?.focus();
      labelOf(activeId)?.scrollIntoView?.({ block: "nearest", inline: "nearest" });
    },
  };
}
