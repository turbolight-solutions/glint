/** Parsing of OIDs typed or pasted into the lookup page. */

export interface OidArc {
  name?: string;
  number: string;
}

export interface ParsedOid {
  oid: string;
  arcs: OidArc[];
  /** Arc names that disagree with the registered name for that arc. */
  warnings: string[];
}

/**
 * Registered names of well-known arcs, keyed by dotted OID. The first name is the one quoted in
 * warnings; the rest are accepted spellings. Names are compared without case, hyphens or a
 * leading `id-`, so `pkix`, `id-pkix` and `Id-PKIX` all match.
 */
const KNOWN_ARCS: Record<string, string[]> = {
  "0": ["itu-t", "ccitt", "itu-r"],
  "1": ["iso"],
  "2": ["joint-iso-itu-t", "joint-iso-ccitt"],
  "1.0": ["standard"],
  "1.2": ["member-body"],
  "1.3": ["identified-organization", "org"],
  "1.2.840": ["us"],
  "1.2.840.10040": ["x9-57", "x9cm"],
  "1.2.840.10045": ["ansi-x9-62", "x9-62"],
  "1.2.840.113533": ["nsn"],
  "1.2.840.113549": ["rsadsi"],
  "1.2.840.113549.1": ["pkcs"],
  "1.2.840.113549.1.1": ["pkcs-1"],
  "1.2.840.113549.1.5": ["pkcs-5"],
  "1.2.840.113549.1.7": ["pkcs-7"],
  "1.2.840.113549.1.9": ["pkcs-9"],
  "1.2.840.113549.1.12": ["pkcs-12"],
  "1.2.840.113549.2": ["digestAlgorithm"],
  "1.2.840.113549.3": ["encryptionAlgorithm"],
  "1.3.6": ["dod"],
  "1.3.6.1": ["internet"],
  "1.3.6.1.1": ["directory"],
  "1.3.6.1.2": ["mgmt"],
  "1.3.6.1.3": ["experimental"],
  "1.3.6.1.4": ["private"],
  "1.3.6.1.4.1": ["enterprise", "enterprises"],
  "1.3.6.1.5": ["security"],
  "1.3.6.1.5.5": ["mechanisms"],
  "1.3.6.1.5.5.7": ["pkix"],
  "1.3.6.1.5.5.7.0": ["mod"],
  "1.3.6.1.5.5.7.1": ["pe"],
  "1.3.6.1.5.5.7.2": ["qt"],
  "1.3.6.1.5.5.7.3": ["kp"],
  "1.3.6.1.5.5.7.4": ["it"],
  "1.3.6.1.5.5.7.5": ["pkip"],
  "1.3.6.1.5.5.7.6": ["alg"],
  "1.3.6.1.5.5.7.7": ["cmc"],
  "1.3.6.1.5.5.7.8": ["on"],
  "1.3.6.1.5.5.7.9": ["pda"],
  "1.3.6.1.5.5.7.10": ["aca"],
  "1.3.6.1.5.5.7.11": ["qcs"],
  "1.3.6.1.5.5.7.12": ["cct"],
  "1.3.6.1.5.5.7.48": ["ad"],
  "1.3.14": ["oiw"],
  "1.3.36": ["teletrust"],
  "1.3.101": ["thawte"],
  "1.3.132": ["certicom"],
  "1.3.132.0": ["ellipticCurve", "secg-curve"],
  "2.5": ["ds"],
  "2.5.4": ["attributeType"],
  "2.5.6": ["objectClass"],
  "2.5.29": ["ce"],
  "2.16": ["country"],
  "2.16.840": ["us"],
  "2.16.840.1": ["organization", "org"],
  "2.16.840.1.101": ["gov"],
  "2.16.840.1.101.3": ["csor"],
  "2.16.840.1.101.3.4": ["nistAlgorithms"],
  "2.16.840.1.113730": ["netscape"],
  "2.23": ["international-organizations"],
  "2.23.140": ["ca-browser-forum", "cabforum"],
};

function canonicalName(name: string): string {
  return name.toLowerCase().replace(/^id-/, "").replace(/[^a-z0-9]/g, "");
}

/** Stripped of everything that is not a visible character, including the invisible ones copy and paste brings along. */
function normalize(text: string): string {
  return text
    .replace(/[​-‍⁠﻿­]/g, "")
    .replace(/\s+/g, " ")
    .trim();
}

const ARC = /(?:([A-Za-z][A-Za-z0-9_-]*)\s*\(\s*(\d+)\s*\)|(\d+))\s*/y;

function checkNames(arcs: OidArc[]): string[] {
  const warnings: string[] = [];
  const path: string[] = [];
  for (const arc of arcs) {
    path.push(arc.number);
    if (arc.name === undefined) continue;
    const dotted = path.join(".");
    const known = KNOWN_ARCS[dotted];
    if (!known || known.some((n) => canonicalName(n) === canonicalName(arc.name ?? ""))) continue;
    warnings.push(`${dotted} is ${known[0]}, not ${arc.name}.`);
  }
  return warnings;
}

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
    return { oid, arcs: oid.split(".").map((number) => ({ number })), warnings: [] };
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
  return { oid: arcs.map((a) => a.number).join("."), arcs, warnings: checkNames(arcs) };
}
