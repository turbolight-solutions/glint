import { describe, expect, it } from "vitest";
import type { ArtifactRow } from "./api";
import { nextArtifactSort, parseArtifactSort, sortArtifacts } from "./artifactsort";

function row(id: string, extra: Partial<ArtifactRow> = {}): ArtifactRow {
  return { id, document: id.replace(/-\d+$/, ""), line: 1, label: "CERTIFICATE", confidence: "label", size: 100, ...extra };
}

const ids = (rows: ArtifactRow[]) => rows.map((r) => r.id);

describe("sorting artifacts", () => {
  it("compares the numbers in example ids as numbers", () => {
    const rows = [row("rfc9500-10"), row("rfc9500-2"), row("rfc8410-7")];
    expect(ids(sortArtifacts(rows, { key: "example", direction: "asc" }))).toEqual(["rfc8410-7", "rfc9500-2", "rfc9500-10"]);
  });

  it("ranks findings by severity, an example with errors as an error", () => {
    const rows = [
      row("clean"),
      row("note", { findings: ["I406"] }),
      row("bad", { findings: ["W408"], first_error: "E401 ..." }),
      row("warned", { findings: ["W402"] }),
    ];
    expect(ids(sortArtifacts(rows, { key: "findings", direction: "desc" }))).toEqual(["bad", "warned", "note", "clean"]);
  });

  it("puts examples with no type last either way", () => {
    const rows = [row("none", { root: undefined }), row("b", { root: "rfc5912#M.B" }), row("a", { root: "rfc5912#M.A" })];
    expect(ids(sortArtifacts(rows, { key: "type", direction: "asc" }))).toEqual(["a", "b", "none"]);
    expect(ids(sortArtifacts(rows, { key: "type", direction: "desc" }))).toEqual(["b", "a", "none"]);
  });

  it("sorts sizes and confidence", () => {
    const rows = [row("s", { size: 5 }), row("l", { size: 500, confidence: "unknown" }), row("m", { size: 50, confidence: "decoded" })];
    expect(ids(sortArtifacts(rows, { key: "size", direction: "desc" }))).toEqual(["l", "m", "s"]);
    expect(ids(sortArtifacts(rows, { key: "confidence", direction: "asc" }))).toEqual(["s", "m", "l"]);
  });

  it("cycles a column through its first direction, the other, and none", () => {
    const first = nextArtifactSort(null, "size");
    expect(first).toEqual({ key: "size", direction: "desc" });
    const second = nextArtifactSort(first, "size");
    expect(second).toEqual({ key: "size", direction: "asc" });
    expect(nextArtifactSort(second, "size")).toBeNull();
    expect(parseArtifactSort("-findings")).toEqual({ key: "findings", direction: "desc" });
    expect(parseArtifactSort("nonsense")).toBeNull();
  });
});
