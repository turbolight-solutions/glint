/** Parsing of OIDs typed or pasted into the lookup page. */

export interface OidArc {
  name?: string;
  number: string;
}

export interface ParsedOid {
  oid: string;
  arcs: OidArc[];
}

/** Stripped of everything that is not a visible character, including the invisible ones copy and paste brings along. */
function normalize(text: string): string {
  return text
    .replace(/[​-‍⁠﻿­]/g, "")
    .replace(/\s+/g, " ")
    .trim();
}

const ARC = /(?:([A-Za-z][A-Za-z0-9_-]*)\s*\(\s*(\d+)\s*\)|(\d+))\s*/y;

/**
 * Reads an OID written as dotted digits (`1.3.6.1.5.5.7`), as ASN.1 value notation with names
 * and numbers (`{ iso(1) identified-organization(3) dod(6) }`), or as a bare list of arcs
 * (`1 3 6 1`). Whitespace of any kind, including line breaks, is ignored. Returns null if the
 * text is none of these.
 */
export function parseOid(text: string): ParsedOid | null {
  let value = normalize(text);
  const braced = /^\{(.*)\}$/.exec(value);
  if (braced?.[1] !== undefined) value = braced[1].trim();
  if (value === "") return null;

  if (/^\d+(?:\s*\.\s*\d+)*$/.test(value)) {
    const oid = value.replace(/\s+/g, "");
    return { oid, arcs: oid.split(".").map((number) => ({ number })) };
  }

  const arcs: OidArc[] = [];
  let at = 0;
  while (at < value.length) {
    ARC.lastIndex = at;
    const match = ARC.exec(value);
    if (!match) return null;
    arcs.push(match[1] === undefined ? { number: match[3] ?? "" } : { name: match[1], number: match[2] ?? "" });
    at = ARC.lastIndex;
  }
  return { oid: arcs.map((a) => a.number).join("."), arcs };
}
