// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { createCombobox, matching } from "./combobox";

describe("matching", () => {
  const values = ["Certificate", "TBSCertificate", "CertificateList", "Name"];

  it("offers everything for an empty query", () => {
    expect(matching(values, " ")).toEqual(values);
  });

  it("puts prefix matches before others, ignoring case", () => {
    expect(matching(values, "cert")).toEqual(["Certificate", "CertificateList", "TBSCertificate"]);
  });

  it("offers nothing when nothing matches", () => {
    expect(matching(values, "Extension")).toEqual([]);
  });
});

function setup() {
  const combo = createCombobox({ id: "t", placeholder: "Type", label: "Type", toggleLabel: "Show types" });
  document.body.replaceChildren(combo.element);
  combo.setOptions(["Certificate", "TBSCertificate", "Name"]);
  const changes: string[] = [];
  combo.input.addEventListener("change", () => changes.push(combo.input.value));
  const menu = combo.element.querySelector<HTMLElement>("[role=listbox]")!;
  const items = () => [...menu.querySelectorAll<HTMLElement>("[role=option]")].map((o) => o.textContent);
  const toggle = combo.element.querySelector<HTMLButtonElement>("button")!;
  const key = (k: string) => combo.input.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true }));
  return { combo, changes, menu, items, toggle, key };
}

describe("createCombobox", () => {
  it("shows every option from the toggle, whatever the input holds", () => {
    const { combo, menu, items, toggle } = setup();
    combo.input.value = "Name";
    toggle.click();
    expect(menu.classList.contains("show")).toBe(true);
    expect(items()).toEqual(["Certificate", "TBSCertificate", "Name"]);
    expect(combo.input.getAttribute("aria-expanded")).toBe("true");
    toggle.click();
    expect(menu.classList.contains("show")).toBe(false);
  });

  const type = (combo: { input: HTMLInputElement }, text: string, inputType = "insertText") => {
    combo.input.value = text;
    combo.input.setSelectionRange(text.length, text.length);
    combo.input.dispatchEvent(new InputEvent("input", { inputType }));
  };

  it("filters as you type and chooses with the keyboard", () => {
    const { combo, changes, items, key } = setup();
    type(combo, "cert");
    expect(items()).toEqual(["Certificate", "TBSCertificate"]);
    key("ArrowDown");
    key("Enter");
    expect(combo.input.value).toBe("TBSCertificate");
    expect(changes).toEqual(["TBSCertificate"]);
  });

  it("fills in the first match, selected, and takes it on Enter", () => {
    const { combo, changes, menu, key } = setup();
    type(combo, "cert");
    expect(combo.input.value).toBe("Certificate");
    expect([combo.input.selectionStart, combo.input.selectionEnd]).toEqual([4, 11]);
    expect(menu.querySelector(".active")?.textContent).toBe("Certificate");
    key("Enter");
    expect(changes).toEqual(["Certificate"]);
    expect(menu.classList.contains("show")).toBe(false);
  });

  it("takes the marked match on Tab", () => {
    const { combo, changes, key } = setup();
    type(combo, "tbs");
    key("Tab");
    expect(changes).toEqual(["TBSCertificate"]);
  });

  it("marks but does not fill in while deleting", () => {
    const { combo, menu } = setup();
    type(combo, "Cer", "deleteContentBackward");
    expect(combo.input.value).toBe("Cer");
    expect(menu.querySelector(".active")?.textContent).toBe("Certificate");
  });

  it("fills in only a prefix match, and Escape takes it back", () => {
    const { combo, key } = setup();
    type(combo, "ert");
    expect(combo.input.value).toBe("ert");
    type(combo, "Na");
    expect(combo.input.value).toBe("Name");
    key("Escape");
    expect(combo.input.value).toBe("Na");
  });

  it("marks nothing for an empty input", () => {
    const { combo, menu } = setup();
    type(combo, "", "deleteContentBackward");
    expect(menu.classList.contains("show")).toBe(true);
    expect(menu.querySelector(".active")).toBeNull();
  });

  it("chooses on click and closes on Escape", () => {
    const { changes, menu, toggle, key } = setup();
    toggle.click();
    menu.querySelectorAll<HTMLElement>("[role=option]")[2]?.click();
    expect(changes).toEqual(["Name"]);
    expect(menu.classList.contains("show")).toBe(false);
    key("ArrowDown");
    expect(menu.classList.contains("show")).toBe(true);
    key("Escape");
    expect(menu.classList.contains("show")).toBe(false);
  });
});
