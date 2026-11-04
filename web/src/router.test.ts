import { describe, expect, it } from "vitest";
import { corpusHash, hashForTarget, moduleHash, parseHash } from "./router";

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
  });

  it("keeps the corpus sort in the URL", () => {
    expect(corpusHash("pkix", { key: "status", direction: "desc" })).toBe("#corpus?q=pkix&sort=-status");
    expect(corpusHash("", { key: "oid", direction: "asc" })).toBe("#corpus?sort=oid");
    expect(corpusHash("pkix")).toBe("#corpus?q=pkix");
    expect(parseHash("#corpus?q=pkix&sort=-status")).toEqual({ view: "corpus", q: "pkix", sort: { key: "status", direction: "desc" } });
    expect(parseHash("#corpus?sort=oid")).toEqual({ view: "corpus", q: "", sort: { key: "oid", direction: "asc" } });
    expect(parseHash("#corpus?sort=bogus")).toEqual({ view: "corpus", q: "" });
  });

  it("reads the module a checker link points at", () => {
    expect(parseHash(moduleHash("rfc5280", "PKIX1 Explicit"))).toEqual({ view: "checker", rfc: 5280, module: "PKIX1 Explicit" });
    expect(parseHash("#draft-a-b-01?module=M")).toEqual({ view: "checker", draft: "draft-a-b-01", module: "M" });
  });

  it("tells two modules of one name apart by OID", () => {
    const oid = "1.3.6.1.5.5.11.0.1.1";
    expect(moduleHash("rfc4998", "ERS")).toBe("#rfc4998?module=ERS");
    expect(parseHash(moduleHash("rfc4998", "ERS", oid))).toEqual({ view: "checker", rfc: 4998, module: "ERS", moduleOid: oid });
    expect(parseHash("#draft-a-b-01?module=M&oid=1.2.3")).toEqual({ view: "checker", draft: "draft-a-b-01", module: "M", moduleOid: "1.2.3" });
  });

  it("opens a module of another standards body as a document of its own", () => {
    expect(moduleHash("X.509", "Itu1")).toBe("#X.509?module=Itu1");
    expect(parseHash(moduleHash("X.509", "Itu1", "2.5.1.1.7"))).toEqual({ view: "checker", document: "X.509", module: "Itu1", moduleOid: "2.5.1.1.7" });
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
