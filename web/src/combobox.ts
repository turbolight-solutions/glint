import { h, replace } from "./dom";

export interface Combobox {
  element: HTMLElement;
  input: HTMLInputElement;
  /** Replaces the options, closing the list if it was open. */
  setOptions(values: readonly string[]): void;
  /** What the list says when there are no options at all, such as why. */
  setEmptyText(text: string): void;
}

export interface ComboboxOptions {
  id: string;
  placeholder: string;
  label: string;
  /** What the toggle button says to a screen reader. */
  toggleLabel: string;
  /** What the list says when there are no options at all. */
  emptyText: string;
  /** What the list says when no option holds what was typed. */
  noMatchText: string;
}

/** The values that hold `query`, ignoring case: those starting with it first, each group in its given order. */
export function matching(values: readonly string[], query: string): string[] {
  const q = query.trim().toLowerCase();
  if (!q) return [...values];
  const starts: string[] = [];
  const contains: string[] = [];
  for (const value of values) {
    const at = value.toLowerCase().indexOf(q);
    if (at === 0) starts.push(value);
    else if (at > 0) contains.push(value);
  }
  return [...starts, ...contains];
}

/**
 * A text input with a list of options under it, as a datalist would give, but with a button
 * that shows every option whatever the input holds. Typing marks the first match and fills in
 * the rest of it, selected, so Enter or Tab takes it and typing on replaces it. Choosing an
 * option puts it in the input and fires `change` there, as typing and leaving it does.
 */
export function createCombobox(options: ComboboxOptions): Combobox {
  let values: readonly string[] = [];
  let shown: string[] = [];
  let active = -1;
  let emptyText = options.emptyText;

  const listId = `${options.id}-list`;
  const input = h("input", {
    class: "form-control form-control-sm",
    type: "text",
    id: options.id,
    placeholder: options.placeholder,
    "aria-label": options.label,
    autocomplete: "off",
    spellcheck: "false",
    role: "combobox",
    "aria-autocomplete": "both",
    "aria-expanded": "false",
    "aria-controls": listId,
  });
  const list = h("div", { class: "dropdown-menu combobox-menu", id: listId, role: "listbox", "aria-label": options.label });
  const toggle = h("button", {
    class: "btn btn-sm btn-outline-secondary dropdown-toggle combobox-toggle",
    type: "button",
    tabindex: -1,
    "aria-label": options.toggleLabel,
    title: options.toggleLabel,
  });
  const element = h("div", { class: "input-group input-group-sm flex-nowrap combobox" }, input, toggle, list);

  const isOpen = () => list.classList.contains("show");

  function setActive(index: number): void {
    list.children[active]?.classList.remove("active");
    active = index;
    const item = list.children[active];
    if (item) {
      item.classList.add("active");
      input.setAttribute("aria-activedescendant", item.id);
      item.scrollIntoView?.({ block: "nearest" });
    } else input.removeAttribute("aria-activedescendant");
  }

  /** Opens the list on the options holding `query`, marking the input's value if it is one; with none, says why. */
  function open(query: string): void {
    shown = matching(values, query);
    active = -1;
    replace(
      list,
      ...shown.map((value, i) =>
        h(
          "div",
          {
            class: "dropdown-item",
            id: `${listId}-${i}`,
            role: "option",
            "aria-selected": String(value === input.value),
            onclick: () => choose(value),
          },
          value,
        ),
      ),
    );
    if (shown.length === 0) replace(list, h("div", { class: "dropdown-item-text small text-secondary" }, values.length ? options.noMatchText : emptyText));
    list.classList.add("show");
    input.setAttribute("aria-expanded", "true");
    const current = shown.indexOf(input.value);
    if (current >= 0) setActive(current);
  }

  function close(): void {
    list.classList.remove("show");
    input.setAttribute("aria-expanded", "false");
    input.removeAttribute("aria-activedescendant");
    active = -1;
  }

  /** Fires `change` even when the input already holds `value`, since completing it put it there unannounced. */
  function choose(value: string): void {
    close();
    input.value = value;
    input.setSelectionRange(value.length, value.length);
    input.dispatchEvent(new Event("change", { bubbles: true }));
  }

  /** Fills in the rest of the first match after what was typed, selected; not while deleting, or Backspace would put it back. */
  function complete(event: Event): void {
    const typed = input.value;
    open(typed);
    if (!typed.trim() || shown.length === 0) return;
    setActive(0);
    const inputEvent = event as InputEvent;
    if (inputEvent.isComposing || inputEvent.inputType?.startsWith("delete")) return;
    const first = shown[0] ?? "";
    if (first.length > typed.length && first.toLowerCase().startsWith(typed.toLowerCase())) {
      input.value = first;
      input.setSelectionRange(typed.length, first.length);
    }
  }

  /** Takes back a completion still selected at the end of the input. */
  function uncomplete(): void {
    const start = input.selectionStart ?? input.value.length;
    if (start < input.value.length && input.selectionEnd === input.value.length) input.value = input.value.slice(0, start);
  }

  // Keep the focus in the input while the list is clicked, so blurring it does not close the list first.
  list.addEventListener("mousedown", (event) => event.preventDefault());
  toggle.addEventListener("mousedown", (event) => event.preventDefault());
  toggle.addEventListener("click", () => {
    if (isOpen()) close();
    else {
      input.focus();
      open("");
    }
  });
  input.addEventListener("input", complete);
  input.addEventListener("blur", close);
  input.addEventListener("keydown", (event) => {
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      if (!isOpen()) {
        open(values.includes(input.value) ? "" : input.value);
        return;
      }
      if (shown.length === 0) return;
      const step = event.key === "ArrowDown" ? 1 : -1;
      setActive(active < 0 ? (step > 0 ? 0 : shown.length - 1) : (active + step + shown.length) % shown.length);
    } else if ((event.key === "Enter" || event.key === "Tab") && isOpen() && active >= 0) {
      // Tab takes the option and still moves the focus on.
      if (event.key === "Enter") event.preventDefault();
      const value = shown[active];
      if (value !== undefined) choose(value);
    } else if (event.key === "Escape" && isOpen()) {
      event.preventDefault();
      uncomplete();
      close();
    }
  });

  return {
    element,
    input,
    setOptions(next) {
      values = [...next];
      close();
    },
    setEmptyText(text) {
      emptyText = text;
    },
  };
}
