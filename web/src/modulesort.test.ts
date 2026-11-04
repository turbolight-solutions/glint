import { describe, expect, it } from "vitest";
import type { ModuleListItem } from "./api";
import { nextSort, sortModules } from "./modulesort";

function mod(name: string, extra: Partial<ModuleListItem> = {}): ModuleListItem {
  return { name, oid: null, body: "rfc", document: "rfc1000", edition: null, url: null, section: null, status: "clean", obsoleted_by: [], ...extra };
}

const names = (list: ModuleListItem[]) => list.map((m) => m.name);

describe("sorting modules", () => {
  const list = [
    mod("clean", { status: "clean" }),
    mod("bad", { status: "errors" }),
    mod("broken", { status: "unparseable" }),
    mod("meh", { status: "warnings" }),
  ];

  it("ranks status by severity", () => {
    expect(names(sortModules(list, { key: "status", direction: "desc" }))).toEqual(["broken", "bad", "meh", "clean"]);
    expect(names(sortModules(list, { key: "status", direction: "asc" }))).toEqual(["clean", "meh", "bad", "broken"]);
  });

  it("compares numbers inside names, sections and OIDs as numbers", () => {
    const modules = [
      mod("M10", { section: "10.1", oid: "1.3.6.1.10" }),
      mod("M9", { section: "9.2", oid: "1.3.6.1.9.1" }),
      mod("M2", { section: "9.10", oid: "1.3.6.1.9" }),
    ];
    expect(names(sortModules(modules, { key: "name", direction: "asc" }))).toEqual(["M2", "M9", "M10"]);
    expect(names(sortModules(modules, { key: "section", direction: "asc" }))).toEqual(["M9", "M2", "M10"]);
    expect(names(sortModules(modules, { key: "oid", direction: "asc" }))).toEqual(["M2", "M9", "M10"]);
  });

  it("puts modules without a value last in either direction", () => {
    const modules = [mod("none"), mod("a", { oid: "1.2" }), mod("b", { oid: "1.3" })];
    expect(names(sortModules(modules, { key: "oid", direction: "asc" }))).toEqual(["a", "b", "none"]);
    expect(names(sortModules(modules, { key: "oid", direction: "desc" }))).toEqual(["b", "a", "none"]);
  });

  it("keeps the incoming order for equal values and leaves the input alone", () => {
    const modules = [mod("z"), mod("a"), mod("m")];
    expect(names(sortModules(modules, { key: "status", direction: "asc" }))).toEqual(["z", "a", "m"]);
    expect(names(sortModules(modules, null))).toEqual(["z", "a", "m"]);
    sortModules(modules, { key: "name", direction: "asc" });
    expect(names(modules)).toEqual(["z", "a", "m"]);
  });

  it("orders documents by title", () => {
    const modules = [mod("a", { document: "rfc5280" }), mod("b", { document: "rfc2459" }), mod("c", { document: "X.509", edition: "10/2012" })];
    expect(names(sortModules(modules, { key: "document", direction: "asc" }))).toEqual(["b", "a", "c"]);
  });
});

describe("header clicks", () => {
  it("cycles first direction, the other direction, then the server's order", () => {
    const first = nextSort(null, "name");
    expect(first).toEqual({ key: "name", direction: "asc" });
    const second = nextSort(first, "name");
    expect(second).toEqual({ key: "name", direction: "desc" });
    expect(nextSort(second, "name")).toBeNull();
  });

  it("starts status with the worst modules", () => {
    expect(nextSort(null, "status")).toEqual({ key: "status", direction: "desc" });
    expect(nextSort({ key: "status", direction: "desc" }, "status")).toEqual({ key: "status", direction: "asc" });
  });

  it("starts over when another column is clicked", () => {
    expect(nextSort({ key: "name", direction: "desc" }, "oid")).toEqual({ key: "oid", direction: "asc" });
  });
});
