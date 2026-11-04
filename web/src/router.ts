export type Route =
  | { view: "checker"; draft?: string; rfc?: number; module?: string; moduleOid?: string }
  | { view: "corpus"; q: string }
  | { view: "module"; source: string; name: string; oid?: string; from?: number; to?: number }
  | { view: "oid"; oid?: string };

const DRAFT = /^draft-[a-z0-9-]+$/;
const RFC = /^rfc(\d{1,5})$/;

/** Reads the URL hash. Pasted text is never part of it. */
export function parseHash(hash: string): Route {
  const raw = hash.replace(/^#\/?/, "");
  const [path = "", query = ""] = raw.split("?", 2);
  const params = new URLSearchParams(query);
  const parts = path.split("/").map((part) => decodeURIComponent(part));
  const first = (parts[0] ?? "").toLowerCase();
  if (first === "corpus") return { view: "corpus", q: params.get("q") ?? "" };
  if (first === "oid") return { view: "oid", oid: parts[1] || undefined };
  if (first === "module" && parts[1] && parts[2]) {
    const from = params.get("from");
    const to = params.get("to");
    return {
      view: "module",
      source: parts[1],
      name: parts[2],
      oid: params.get("oid") || undefined,
      from: from === null ? undefined : Number(from),
      to: to === null ? undefined : Number(to),
    };
  }
  const module = params.get("module") || undefined;
  const moduleOid = params.get("oid") || undefined;
  if (DRAFT.test(first)) return { view: "checker", draft: first, module, moduleOid };
  const rfc = RFC.exec(first);
  if (rfc?.[1]) return { view: "checker", rfc: Number(rfc[1]), module, moduleOid };
  return { view: "checker" };
}

/**
 * A module is named by its source and name. An RFC can define two modules with one name and different OIDs, so
 * the OID says which one.
 */
export function moduleHash(source: string, name: string, range?: { start: number; end: number }, oid?: string | null): string {
  const base = `#module/${encodeURIComponent(source)}/${encodeURIComponent(name)}`;
  const query: string[] = [];
  if (oid) query.push(`oid=${encodeURIComponent(oid)}`);
  if (range) query.push(`from=${range.start}`, `to=${range.end}`);
  return query.length > 0 ? `${base}?${query.join("&")}` : base;
}

/** Opens an RFC in the checker with the cursor at the start of one of its modules. */
export function rfcModuleHash(rfc: number, module: string, oid?: string | null): string {
  const base = `#rfc${rfc}?module=${encodeURIComponent(module)}`;
  return oid ? `${base}&oid=${encodeURIComponent(oid)}` : base;
}

export function oidHash(oid: string): string {
  return `#oid/${encodeURIComponent(oid)}`;
}

export function corpusHash(q: string): string {
  return q ? `#corpus?q=${encodeURIComponent(q)}` : "#corpus";
}

/** Turns what an author types into a hash: `RFC 5280`, `5280`, or a draft name. */
export function hashForTarget(input: string): string | null {
  const text = input.trim().toLowerCase();
  const number = /^(?:rfc)?[\s-]*(\d{1,5})$/.exec(text);
  if (number?.[1]) return `#rfc${Number(number[1])}`;
  const draft = text.replace(/\.(txt|xml)$/, "");
  return DRAFT.test(draft) ? `#${draft}` : null;
}
