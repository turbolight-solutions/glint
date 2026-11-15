// How the workbench shows a decode: tag and value text as `glint decode` prints them, and which TLV owns each byte.
import type { Tlv } from "./api";

const UNIVERSAL: Record<number, string> = {
  0: "EOC",
  1: "BOOLEAN",
  2: "INTEGER",
  3: "BIT STRING",
  4: "OCTET STRING",
  5: "NULL",
  6: "OBJECT IDENTIFIER",
  7: "ObjectDescriptor",
  8: "EXTERNAL",
  9: "REAL",
  10: "ENUMERATED",
  11: "EMBEDDED PDV",
  12: "UTF8String",
  13: "RELATIVE-OID",
  14: "TIME",
  16: "SEQUENCE",
  17: "SET",
  18: "NumericString",
  19: "PrintableString",
  20: "TeletexString",
  21: "VideotexString",
  22: "IA5String",
  23: "UTCTime",
  24: "GeneralizedTime",
  25: "GraphicString",
  26: "VisibleString",
  27: "GeneralString",
  28: "UniversalString",
  29: "CHARACTER STRING",
  30: "BMPString",
  31: "DATE",
  32: "TIME-OF-DAY",
  33: "DATE-TIME",
  34: "DURATION",
  35: "OID-IRI",
  36: "RELATIVE-OID-IRI",
};

/** `SEQUENCE`, `[0] EXPLICIT`, `[APPLICATION 3]`; `(not a TLV)` for octets that do not start one. */
export function tagText(tlv: Tlv): string {
  const tag = tlv.tag;
  if (!tag) return "(not a TLV)";
  let text: string;
  switch (tag.class) {
    case "universal":
      text = UNIVERSAL[tag.number] ?? `[UNIVERSAL ${tag.number}]`;
      break;
    case "application":
      text = `[APPLICATION ${tag.number}]`;
      break;
    case "context":
      text = `[${tag.number}]`;
      break;
    case "private":
      text = `[PRIVATE ${tag.number}]`;
      break;
  }
  if (tlv.tagging === "explicit") text += " EXPLICIT";
  if (tlv.tagging === "implicit") text += " IMPLICIT";
  return text;
}

const HEX_SHOWN = 32;

function elided(hex: string): string {
  return hex.length > HEX_SHOWN * 2 ? `${hex.slice(0, HEX_SHOWN * 2)}… (${hex.length / 2} octets)` : hex;
}

/** Whether the children of a TLV are the value its octet or bit string contains, rather than its segments. */
export function holdsContents(tlv: Tlv): boolean {
  return (tlv.children?.length ?? 0) > 0 && tlv.tag !== undefined && !tlv.tag.constructed;
}

/** The value of a TLV in one line, or null for one without (constructed, `NULL`). */
export function valueText(tlv: Tlv): string | null {
  const value = tlv.value;
  if (!value) return null;
  switch (value.kind) {
    case "boolean":
      return value.value ? "TRUE" : "FALSE";
    case "integer": {
      const text = value.hex ? `0x${elided(value.hex)}` : value.decimal;
      return value.name ? `${text} (${value.name})` : text;
    }
    case "enumerated":
      return value.name ? `${value.decimal} (${value.name})` : value.decimal;
    case "null":
      return null;
    case "bit-string":
      if (value.named && value.named.length > 0) return value.named.join(", ");
      return holdsContents(tlv) ? "CONTAINING" : `${value.unused} unused bits: ${elided(value.hex)}`;
    case "octet-string":
      return holdsContents(tlv) ? "CONTAINING" : elided(value.hex);
    case "oid": {
      const name = value.names?.[0];
      return name ? `${value.dotted} ${name.name}` : value.dotted;
    }
    case "relative-oid":
      return value.dotted;
    case "time":
      return value.normalized ? `${value.text} (${value.normalized})` : value.text;
    case "string":
      return JSON.stringify(value.text).slice(1, -1);
    case "real":
      return value.text;
    case "hex":
      return elided(value.hex);
  }
}

/** `Type` of `Module.Type`, keeping actual parameters, which may hold dots. */
export function shortType(name: string): string {
  const baseEnd = name.includes("{") ? name.indexOf("{") : name.length;
  const dot = name.slice(0, baseEnd).indexOf(".");
  return dot < 0 ? name : name.slice(dot + 1);
}

/** Every TLV of a tree, depth first: a TLV's index in this list is how the tree and hex views name it. */
export function flatten(tlvs: readonly Tlv[]): Tlv[] {
  const out: Tlv[] = [];
  const visit = (tlv: Tlv): void => {
    out.push(tlv);
    for (const child of tlv.children ?? []) visit(child);
  };
  for (const tlv of tlvs) visit(tlv);
  return out;
}

/** For each of `length` bytes, the index in `nodes` of the innermost TLV that holds it, or -1. */
export function byteOwners(nodes: readonly Tlv[], length: number): Int32Array {
  const owners = new Int32Array(length).fill(-1);
  // Depth first, so an inner TLV, which comes after the TLVs around it, writes last.
  nodes.forEach((tlv, index) => {
    owners.fill(index, Math.max(0, tlv.offset), Math.min(length, tlv.end));
  });
  return owners;
}

/** The octets of base64 text. */
export function fromBase64(text: string): Uint8Array {
  const binary = atob(text);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

/** `1.2 KB`. */
export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}
