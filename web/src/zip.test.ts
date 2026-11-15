import { describe, expect, it } from "vitest";
import { crc32, zip } from "./zip";

describe("zip", () => {
  it("computes the CRC-32 of the zip format", () => {
    expect(crc32(new TextEncoder().encode("123456789"))).toBe(0xcbf43926);
  });

  it("stores each file after a local header, then the central directory", () => {
    const archive = zip([
      { path: "a.rs", contents: "fn a() {}\n" },
      { path: "glint/b.rs", contents: "" },
    ]);
    const view = new DataView(archive.buffer);
    expect(view.getUint32(0, true)).toBe(0x04034b50);
    const end = archive.length - 22;
    expect(view.getUint32(end, true)).toBe(0x06054b50);
    expect(view.getUint16(end + 10, true)).toBe(2);
    const centralOffset = view.getUint32(end + 16, true);
    expect(view.getUint32(centralOffset, true)).toBe(0x02014b50);
    const text = new TextDecoder().decode(archive);
    expect(text).toContain("fn a() {}");
    expect(text).toContain("glint/b.rs");
  });
});
