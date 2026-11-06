import { describe, expect, it } from "vitest";
import { parseOid } from "./oid";

const PKIX = "1.3.6.1.5.5.7";

describe("parseOid", () => {
  it("accepts dotted digits", () => {
    expect(parseOid(PKIX)?.oid).toBe(PKIX);
    expect(parseOid("  1.3.6.1 \n")?.oid).toBe("1.3.6.1");
  });

  it("strips whitespace inside a dotted OID", () => {
    expect(parseOid("1.3.6.1.\n5.5 .7")?.oid).toBe(PKIX);
  });

  it("accepts names with numbers in parentheses", () => {
    const parsed = parseOid("iso(1) identified-organization(3) dod(6) internet(1) security(5) mechanisms(5) pkix(7)");
    expect(parsed?.oid).toBe(PKIX);
    expect(parsed?.arcs.map((a) => a.name)).toEqual(["iso", "identified-organization", "dod", "internet", "security", "mechanisms", "pkix"]);
  });

  it("accepts value notation in braces across lines, tabs and odd spaces", () => {
    const parsed = parseOid("{ iso(1)\n\tidentified-organization (3) dod( 6 )​internet(1) 5 5 pkix(7) }");
    expect(parsed?.oid).toBe(PKIX);
  });

  it("accepts the multi-line braced form from RFC module headers", () => {
    const parsed = parseOid("{ iso(1) identified-organization(3) dod(6) internet(1)\n                    security(5) mechanisms(5) pkix(7) }");
    expect(parsed?.oid).toBe(PKIX);
  });

  it("accepts a bare list of arcs", () => {
    expect(parseOid("1 3 6 1 5 5 7")?.oid).toBe(PKIX);
  });

  it("keeps large arcs exact", () => {
    expect(parseOid("joint-iso-itu-t(2) 16 840 1 101 3 4 2 1234567890123456789012")?.oid).toBe("2.16.840.1.101.3.4.2.1234567890123456789012");
  });

  it("keeps arcs written as bare numbers unnamed", () => {
    expect(parseOid("iso(1) 3 6")?.arcs).toEqual([{ name: "iso", number: "1" }, { number: "3" }, { number: "6" }]);
  });

  it("rejects text that is not an OID", () => {
    expect(parseOid("")).toBeNull();
    expect(parseOid("pkix")).toBeNull();
    expect(parseOid("1.3.x")).toBeNull();
    expect(parseOid("1..3")).toBeNull();
    expect(parseOid("iso(1) bogus")).toBeNull();
  });
});
