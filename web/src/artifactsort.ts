import type { ArtifactRow, Confidence } from "./api";
import type { SortDirection } from "./modulesort";

export const ARTIFACT_SORT_KEYS = ["example", "label", "type", "confidence", "size", "findings"] as const;
export type ArtifactSortKey = (typeof ARTIFACT_SORT_KEYS)[number];
export interface ArtifactSort {
  key: ArtifactSortKey;
  direction: SortDirection;
}

const text = new Intl.Collator("en", { numeric: true, sensitivity: "base" });

/** How sure the identification is: by the label first. */
const CONFIDENCE_RANK: Record<Confidence, number> = { label: 0, decoded: 1, unknown: 2 };

/** A sort as it goes in the URL: `size` for ascending, `-size` for descending. */
export function formatArtifactSort(sort: ArtifactSort): string {
  return `${sort.direction === "desc" ? "-" : ""}${sort.key}`;
}

/** Reads what `formatArtifactSort` wrote. Anything that is not a column is no sort. */
export function parseArtifactSort(value: string | null | undefined): ArtifactSort | null {
  if (!value) return null;
  const descending = value.startsWith("-");
  const key = ARTIFACT_SORT_KEYS.find((k) => k === (descending ? value.slice(1) : value));
  return key ? { key, direction: descending ? "desc" : "asc" } : null;
}

/** The direction a column sorts in on its first click: the worst findings and the largest first, the rest A to Z. */
export function firstArtifactDirection(key: ArtifactSortKey): SortDirection {
  return key === "findings" || key === "size" ? "desc" : "asc";
}

/** The sort a click on a column header leads to: first direction, then the other, then back to the data's order. */
export function nextArtifactSort(current: ArtifactSort | null, key: ArtifactSortKey): ArtifactSort | null {
  if (current?.key !== key) return { key, direction: firstArtifactDirection(key) };
  if (current.direction === firstArtifactDirection(key)) return { key, direction: current.direction === "asc" ? "desc" : "asc" };
  return null;
}

/** The identified type as the table shows it: `Module.Type`, without the document. */
export function artifactType(row: ArtifactRow): string | null {
  return row.root ? (row.root.split("#").pop() ?? row.root) : null;
}

/** How bad an example's findings are: an error, then a warning, then a note; an example with errors counts as one. */
export function findingsSeverity(row: ArtifactRow): number {
  let worst = row.first_error && row.confidence !== "unknown" ? 3 : 0;
  for (const code of row.findings ?? []) worst = Math.max(worst, code.startsWith("E") ? 3 : code.startsWith("W") ? 2 : 1);
  return worst;
}

/**
 * Sorts a copy of `rows`. An example with no type goes last whichever way the column runs, and equal values keep the
 * order they came in, which is the data's (by document, then position).
 */
export function sortArtifacts(rows: readonly ArtifactRow[], sort: ArtifactSort | null): ArtifactRow[] {
  const copy = [...rows];
  if (!sort) return copy;
  const sign = sort.direction === "asc" ? 1 : -1;
  copy.sort((a, b) => {
    switch (sort.key) {
      case "example":
        return sign * text.compare(a.id, b.id);
      case "label":
        return sign * text.compare(a.label, b.label);
      case "type": {
        const left = artifactType(a);
        const right = artifactType(b);
        if (!left || !right) return left ? -1 : right ? 1 : 0;
        return sign * text.compare(left, right);
      }
      case "confidence":
        return sign * (CONFIDENCE_RANK[a.confidence] - CONFIDENCE_RANK[b.confidence]);
      case "size":
        return sign * (a.size - b.size);
      case "findings":
        return sign * (findingsSeverity(a) - findingsSeverity(b) || (a.findings?.length ?? 0) - (b.findings?.length ?? 0));
    }
  });
  return copy;
}
