import { type ModuleListItem, documentTitle } from "./api";

export const SORT_KEYS = ["name", "document", "section", "oid", "status"] as const;
export type SortKey = (typeof SORT_KEYS)[number];
export type SortDirection = "asc" | "desc";
export interface Sort {
  key: SortKey;
  direction: SortDirection;
}

/** How bad a module's check result is. A module that did not parse could not be checked at all, so it ranks worst. */
export const STATUS_SEVERITY: Record<ModuleListItem["status"], number> = {
  clean: 0,
  warnings: 1,
  errors: 2,
  unparseable: 3,
};

const text = new Intl.Collator("en", { numeric: true, sensitivity: "base" });

function compareOid(a: string, b: string): number {
  const left = a.split(".");
  const right = b.split(".");
  for (let i = 0; i < Math.min(left.length, right.length); i++) {
    const difference = Number(left[i]) - Number(right[i]);
    if (difference !== 0 && !Number.isNaN(difference)) return difference;
  }
  return left.length - right.length;
}

/** A sort as it goes in the URL: `status` for ascending, `-status` for descending. */
export function formatSort(sort: Sort): string {
  return `${sort.direction === "desc" ? "-" : ""}${sort.key}`;
}

/** Reads what `formatSort` wrote. Anything that is not a column is no sort. */
export function parseSort(text: string | null | undefined): Sort | null {
  if (!text) return null;
  const descending = text.startsWith("-");
  const key = SORT_KEYS.find((k) => k === (descending ? text.slice(1) : text));
  return key ? { key, direction: descending ? "desc" : "asc" } : null;
}

/** The direction a column sorts in on its first click: worst status first, everything else A to Z. */
export function firstDirection(key: SortKey): SortDirection {
  return key === "status" ? "desc" : "asc";
}

/** The sort a click on a column header leads to: first direction, then the other, then back to the server's order. */
export function nextSort(current: Sort | null, key: SortKey): Sort | null {
  if (current?.key !== key) return { key, direction: firstDirection(key) };
  if (current.direction === firstDirection(key)) return { key, direction: current.direction === "asc" ? "desc" : "asc" };
  return null;
}

/**
 * Sorts a copy of `modules`. Modules without a value (no section, no OID) go last whichever way the column
 * runs, and equal values keep the order they came in, which is the server's.
 */
export function sortModules(modules: readonly ModuleListItem[], sort: Sort | null): ModuleListItem[] {
  const copy = [...modules];
  if (!sort) return copy;
  const sign = sort.direction === "asc" ? 1 : -1;
  copy.sort((a, b) => {
    switch (sort.key) {
      case "name":
        return sign * text.compare(a.name, b.name);
      case "document":
        return sign * text.compare(documentTitle(a.document, a.edition), documentTitle(b.document, b.edition));
      case "status":
        return sign * (STATUS_SEVERITY[a.status] - STATUS_SEVERITY[b.status]);
      case "section":
      case "oid": {
        const left = sort.key === "section" ? a.section : a.oid;
        const right = sort.key === "section" ? b.section : b.oid;
        if (!left || !right) return left ? -1 : right ? 1 : 0;
        return sign * (sort.key === "oid" ? compareOid(left, right) : text.compare(left, right));
      }
    }
  });
  return copy;
}
