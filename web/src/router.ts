import { type Sort, formatSort, parseSort } from "./modulesort";

/**
 * A checker route names one document to open. `draft` and `rfc` are documents the server fetches whole;
 * `document` is any other corpus document, whose modules are files of their own, so it comes with the
 * `module` to open. For a draft or RFC, `module` says where the cursor goes once the document is open. `xml` opens the
 * RFCXML of a draft or RFC that the corpus has as its text, an RFC's corrected by its errata (`#rfc10082.xml`,
 * `#draft-ietf-lamps-foo-03.xml`), instead of its text (`#rfc10082`, `#draft-ietf-lamps-foo-03`).
 */
export type Route =
  | { view: "checker"; draft?: string; rfc?: number; xml?: boolean; document?: string; module?: string; moduleOid?: string }
  | { view: "corpus"; q: string; sort?: Sort }
  | { view: "oid"; oid?: string };

const DRAFT = /^draft-[a-z0-9-]+$/;
const DRAFT_VIEW = /^(draft-[a-z0-9-]+)(\.xml)?$/;
const RFC = /^rfc(\d{1,5})(\.xml)?$/;

/** Reads the URL hash. Pasted text is never part of it. */
export function parseHash(hash: string): Route {
  const raw = hash.replace(/^#\/?/, "");
  const [path = "", query = ""] = raw.split("?", 2);
  const params = new URLSearchParams(query);
  const parts = path.split("/").map((part) => decodeURIComponent(part));
  const first = (parts[0] ?? "").toLowerCase();
  if (first === "corpus") return { view: "corpus", q: params.get("q") ?? "", sort: parseSort(params.get("sort")) ?? undefined };
  if (first === "oid") return { view: "oid", oid: parts[1] || undefined };
  const module = params.get("module") || undefined;
  const moduleOid = params.get("oid") || undefined;
  const draft = DRAFT_VIEW.exec(first);
  if (draft?.[1]) return { view: "checker", draft: draft[1], ...(draft[2] ? { xml: true } : {}), module, moduleOid };
  const rfc = RFC.exec(first);
  if (rfc?.[1]) return { view: "checker", rfc: Number(rfc[1]), ...(rfc[2] ? { xml: true } : {}), module, moduleOid };
  const document = parts[0];
  if (document && module) return { view: "checker", document, module, moduleOid };
  return { view: "checker" };
}

/**
 * Where a corpus module opens: in the checker, with the cursor at the module's start, whatever body it comes
 * from. An RFC can define two modules with one name and different OIDs, so the OID says which one.
 */
export function moduleHash(source: string, name: string, oid?: string | null): string {
  const base = `#${encodeURIComponent(source)}?module=${encodeURIComponent(name)}`;
  return oid ? `${base}&oid=${encodeURIComponent(oid)}` : base;
}

export function oidHash(oid: string): string {
  return `#oid/${encodeURIComponent(oid)}`;
}

export function corpusHash(q: string, sort?: Sort | null): string {
  const params = [q ? `q=${encodeURIComponent(q)}` : "", sort ? `sort=${encodeURIComponent(formatSort(sort))}` : ""].filter(Boolean);
  return params.length > 0 ? `#corpus?${params.join("&")}` : "#corpus";
}

/** Where an RFC opens: its text, or its corrected RFCXML. */
export function rfcHash(rfc: number, xml?: boolean): string {
  return `#rfc${rfc}${xml ? ".xml" : ""}`;
}

/** Where a draft opens: its text, or its RFCXML. */
export function draftHash(draft: string, xml?: boolean): string {
  return `#${draft}${xml ? ".xml" : ""}`;
}

/** Turns what an author types into a hash: `RFC 5280`, `5280`, `rfc10082.xml`, or a draft name (`.xml` for its RFCXML). */
export function hashForTarget(input: string): string | null {
  const text = input.trim().toLowerCase();
  const number = /^(?:rfc)?[\s-]*(\d{1,5})(\.txt|\.xml)?$/.exec(text);
  if (number?.[1]) return rfcHash(Number(number[1]), number[2] === ".xml");
  const draft = /^(.*?)(\.txt|\.xml)?$/.exec(text);
  return draft?.[1] && DRAFT.test(draft[1]) ? draftHash(draft[1], draft[2] === ".xml") : null;
}
