import { describe, expect, it } from "vitest";
import type { Tlv } from "./api";
import { byteOwners, flatten, formatSize, fromBase64, shortType, tagText, valueText } from "./tlv";

function tlv(fields: Partial<Tlv>): Tlv {
  return { offset: 0, header_len: 2, len: { form: "definite", octets: 0 }, end: 2, schema: { state: "matched" }, ...fields };
}

describe("tagText", () => {
  it("names universal tags and writes the others as the module does", () => {
    expect(tagText(tlv({ tag: { class: "universal", number: 16, constructed: true } }))).toBe("SEQUENCE");
    expect(tagText(tlv({ tag: { class: "universal", number: 99, constructed: false } }))).toBe("[UNIVERSAL 99]");
    expect(tagText(tlv({ tag: { class: "context", number: 0, constructed: true }, tagging: "explicit" }))).toBe("[0] EXPLICIT");
    expect(tagText(tlv({ tag: { class: "application", number: 3, constructed: false } }))).toBe("[APPLICATION 3]");
    expect(tagText(tlv({}))).toBe("(not a TLV)");
  });
});

describe("valueText", () => {
  it("shows values as glint decode does", () => {
    expect(valueText(tlv({ value: { kind: "integer", decimal: "2", name: "v3" } }))).toBe("2 (v3)");
    expect(valueText(tlv({ value: { kind: "oid", dotted: "2.5.29.15", names: [{ name: "id-ce-keyUsage", module: "M" }] } }))).toBe(
      "2.5.29.15 id-ce-keyUsage",
    );
    expect(valueText(tlv({ value: { kind: "null" } }))).toBeNull();
    expect(valueText(tlv({ value: { kind: "string", text: 'a"b\n' } }))).toBe('a\\"b\\n');
    expect(valueText(tlv({ value: { kind: "octet-string", hex: "AB".repeat(40) } }))).toBe(`${"AB".repeat(32)}… (40 octets)`);
  });

  it("says CONTAINING for an octet string whose contents were decoded", () => {
    const inner = tlv({ offset: 2, end: 4 });
    const outer = tlv({ tag: { class: "universal", number: 4, constructed: false }, value: { kind: "octet-string", hex: "0500" }, children: [inner] });
    expect(valueText(outer)).toBe("CONTAINING");
  });
});

describe("byteOwners", () => {
  it("gives each byte to the innermost TLV that holds it", () => {
    const a = tlv({ offset: 2, end: 5 });
    const b = tlv({ offset: 5, end: 8 });
    const root = tlv({ offset: 0, end: 8, children: [a, b] });
    const trailing = tlv({ offset: 8, end: 9 });
    const nodes = flatten([root, trailing]);
    expect(nodes).toEqual([root, a, b, trailing]);
    expect(Array.from(byteOwners(nodes, 10))).toEqual([0, 0, 1, 1, 1, 2, 2, 2, 3, -1]);
  });
});

describe("helpers", () => {
  it("shortens type names, keeping actual parameters", () => {
    expect(shortType("PKIX1Explicit-2009.Certificate")).toBe("Certificate");
    expect(shortType("M.SIGNED{M.TBSCertificate}")).toBe("SIGNED{M.TBSCertificate}");
    expect(shortType("Plain")).toBe("Plain");
  });

  it("decodes base64 and formats sizes", () => {
    expect(Array.from(fromBase64("MAYCAQUBAf8="))).toEqual([0x30, 6, 2, 1, 5, 1, 1, 0xff]);
    expect(formatSize(512)).toBe("512 B");
    expect(formatSize(2048)).toBe("2.0 KB");
  });
});
