import { describe, expect, it } from "vitest";
import { OffsetMap } from "./offsets";

describe("OffsetMap", () => {
  it("is the identity for ASCII", () => {
    const map = new OffsetMap("abc");
    expect(map.toIndex(0)).toBe(0);
    expect(map.toIndex(2)).toBe(2);
    expect(map.toIndex(3)).toBe(3);
  });

  it("counts multi-byte characters as several bytes and one unit", () => {
    const map = new OffsetMap("aéb€c");
    expect(map.toIndex(0)).toBe(0);
    expect(map.toIndex(1)).toBe(1);
    expect(map.toIndex(3)).toBe(2);
    expect(map.toIndex(4)).toBe(3);
    expect(map.toIndex(7)).toBe(4);
    expect(map.toByte(4)).toBe(7);
  });

  it("counts a surrogate pair as four bytes and two units", () => {
    const map = new OffsetMap("x\u{1f600}y");
    expect(map.toIndex(1)).toBe(1);
    expect(map.toIndex(5)).toBe(3);
    expect(map.toByte(3)).toBe(5);
    expect(map.toIndex(2)).toBe(1);
  });

  it("clamps offsets past the end", () => {
    const map = new OffsetMap("ab");
    expect(map.toIndex(99)).toBe(2);
    expect(map.toByte(99)).toBe(2);
  });
});
