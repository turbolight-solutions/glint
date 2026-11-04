import { describe, expect, it } from "vitest";
import { hashForTarget, moduleHash, parseHash, rfcModuleHash } from "./router";

describe("parseHash", () => {
  it("defaults to the checker", () => {
    expect(parseHash("")).toEqual({ view: "checker" });
    expect(parseHash("#")).toEqual({ view: "checker" });
  });

  it("reads a draft name and an RFC number", () => {
    expect(parseHash("#draft-ietf-lamps-foo-03")).toEqual({ view: "checker", draft: "draft-ietf-lamps-foo-03" });
    expect(parseHash("#rfc5280")).toEqual({ view: "checker", rfc: 5280 });
  });

  it("reads the corpus, module and OID routes", () => {
    expect(parseHash("#corpus?q=pkix")).toEqual({ view: "corpus", q: "pkix" });
    expect(parseHash("#oid/1.3.6.1")).toEqual({ view: "oid", oid: "1.3.6.1" });
    expect(parseHash("#oid")).toEqual({ view: "oid", oid: undefined });
    expect(parseHash(moduleHash("rfc5280", "PKIX1Explicit88", { start: 10, end: 20 }))).toEqual({
      view: "module",
      source: "rfc5280",
      name: "PKIX1Explicit88",
      from: 10,
      to: 20,
    });
  });

  it("reads the module a checker link points at", () => {
    expect(parseHash(rfcModuleHash(5280, "PKIX1 Explicit"))).toEqual({ view: "checker", rfc: 5280, module: "PKIX1 Explicit" });
    expect(parseHash("#draft-a-b-01?module=M")).toEqual({ view: "checker", draft: "draft-a-b-01", module: "M" });
  });

  it("tells two modules of one name apart by OID", () => {
    const oid = "1.3.6.1.5.5.11.0.1.1";
    expect(moduleHash("rfc4998", "ERS")).toBe("#module/rfc4998/ERS");
    expect(parseHash(moduleHash("rfc4998", "ERS", undefined, oid))).toEqual({ view: "module", source: "rfc4998", name: "ERS", oid });
    expect(parseHash(moduleHash("rfc4998", "ERS", { start: 1, end: 2 }, oid))).toEqual({
      view: "module",
      source: "rfc4998",
      name: "ERS",
      oid,
      from: 1,
      to: 2,
    });
    expect(parseHash(rfcModuleHash(4998, "ERS", oid))).toEqual({ view: "checker", rfc: 4998, module: "ERS", moduleOid: oid });
    expect(parseHash("#draft-a-b-01?module=M&oid=1.2.3")).toEqual({ view: "checker", draft: "draft-a-b-01", module: "M", moduleOid: "1.2.3" });
  });

  it("ignores anything that is not a route", () => {
    expect(parseHash("#<script>")).toEqual({ view: "checker" });
  });
});

describe("hashForTarget", () => {
  it("accepts RFC numbers in common spellings", () => {
    for (const input of ["5280", "RFC5280", "rfc 5280", "RFC-5280"]) {
      expect(hashForTarget(input)).toBe("#rfc5280");
    }
  });

  it("accepts draft names, with or without a file extension", () => {
    expect(hashForTarget("draft-ietf-lamps-foo-03.txt")).toBe("#draft-ietf-lamps-foo-03");
    expect(hashForTarget("Draft-IETF-lamps-foo")).toBe("#draft-ietf-lamps-foo");
  });

  it("rejects everything else", () => {
    expect(hashForTarget("hello world")).toBeNull();
    expect(hashForTarget("")).toBeNull();
  });
});
