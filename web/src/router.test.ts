import { describe, expect, it } from "vitest";
import { corpusHash, draftHash, hashForTarget, moduleHash, parseHash, rfcHash, rootRoute, workbenchHash } from "./router";

describe("parseHash", () => {
  it("defaults to the checker", () => {
    expect(parseHash("")).toEqual({ view: "checker" });
    expect(parseHash("#")).toEqual({ view: "checker" });
  });

  it("reads a draft name and an RFC number", () => {
    expect(parseHash("#draft-ietf-lamps-foo-03")).toEqual({ view: "checker", draft: "draft-ietf-lamps-foo-03" });
    expect(parseHash("#rfc5280")).toEqual({ view: "checker", rfc: 5280 });
  });

  it("reads the RFCXML view of an RFC", () => {
    expect(parseHash("#rfc10082.xml")).toEqual({ view: "checker", rfc: 10082, xml: true });
    expect(parseHash("#rfc10082")).not.toHaveProperty("xml");
    expect(parseHash(rfcHash(10082, true))).toEqual({ view: "checker", rfc: 10082, xml: true });
    expect(rfcHash(10082)).toBe("#rfc10082");
  });

  it("reads the RFCXML view of a draft", () => {
    expect(parseHash("#draft-ietf-lamps-foo-03.xml")).toEqual({ view: "checker", draft: "draft-ietf-lamps-foo-03", xml: true });
    expect(parseHash("#draft-ietf-lamps-foo.xml")).toEqual({ view: "checker", draft: "draft-ietf-lamps-foo", xml: true });
    expect(parseHash("#draft-ietf-lamps-foo-03")).not.toHaveProperty("xml");
    expect(parseHash(draftHash("draft-a-b-01", true))).toEqual({ view: "checker", draft: "draft-a-b-01", xml: true });
    expect(draftHash("draft-a-b-01")).toBe("#draft-a-b-01");
    expect(parseHash("#draft-a-b-01.xml?module=M")).toEqual({ view: "checker", draft: "draft-a-b-01", xml: true, module: "M" });
    // Any other extension is no route.
    expect(parseHash("#draft-a-b-01.txt")).toEqual({ view: "checker" });
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

  it("opens an RFC's RFCXML when its file name is typed", () => {
    expect(hashForTarget("rfc10082.xml")).toBe("#rfc10082.xml");
    expect(hashForTarget("RFC10082.txt")).toBe("#rfc10082");
  });

  it("accepts draft names, with or without a file extension, and opens a draft's RFCXML by its file name", () => {
    expect(hashForTarget("draft-ietf-lamps-foo-03.txt")).toBe("#draft-ietf-lamps-foo-03");
    expect(hashForTarget("Draft-IETF-lamps-foo")).toBe("#draft-ietf-lamps-foo");
    expect(hashForTarget("draft-ietf-lamps-foo-03.xml")).toBe("#draft-ietf-lamps-foo-03.xml");
    expect(hashForTarget("draft-ietf-lamps-foo-03.md")).toBeNull();
  });

  it("rejects everything else", () => {
    expect(hashForTarget("hello world")).toBeNull();
    expect(hashForTarget("")).toBeNull();
  });
});

describe("workbench routes", () => {
  it("reads the picked module and type, the tab and the artifact", () => {
    expect(parseHash("#workbench")).toEqual({ view: "workbench", tab: "decode" });
    expect(parseHash("#/workbench/rfc5912/PKIX1Explicit-2009/Certificate")).toEqual({
      view: "workbench",
      tab: "decode",
      source: "rfc5912",
      module: "PKIX1Explicit-2009",
      type: "Certificate",
    });
    expect(parseHash("#workbench?tab=artifacts&document=rfc9500")).toEqual({ view: "workbench", tab: "artifacts", document: "rfc9500" });
    expect(parseHash("#workbench/rfc4998/ERS?oid=1.3.6.1.5.5.11.0.2.1&artifact=rfc4998-1")).toEqual({
      view: "workbench",
      tab: "decode",
      source: "rfc4998",
      module: "ERS",
      moduleOid: "1.3.6.1.5.5.11.0.2.1",
      artifact: "rfc4998-1",
    });
  });

  it("writes hashes that read back", () => {
    const route = { tab: "decode" as const, source: "X.509", module: "Auth Framework", type: "Certificate", artifact: "rfc1-1" };
    const hash = workbenchHash(route);
    expect(hash).toBe("#workbench/X.509/Auth%20Framework/Certificate?artifact=rfc1-1");
    expect(parseHash(hash)).toEqual({ view: "workbench", ...route });
    expect(workbenchHash({ tab: "artifacts", document: "rfc9500" })).toBe("#workbench?tab=artifacts&document=rfc9500");
    expect(workbenchHash({ tab: "decode" })).toBe("#workbench");
  });

  it("splits a root type into the picker's module and type", () => {
    expect(rootRoute("rfc5912#PKIX1Explicit-2009.Certificate")).toEqual({ source: "rfc5912", module: "PKIX1Explicit-2009", type: "Certificate" });
    expect(rootRoute("Mine.T")).toBeNull();
  });
});

