/** Converts between the UTF-8 byte offsets the server reports and the UTF-16 offsets of a JavaScript string. */
export class OffsetMap {
  private readonly byteAt: Uint32Array;

  constructor(readonly text: string) {
    const bytes = new Uint32Array(text.length + 1);
    let total = 0;
    for (let i = 0; i < text.length; ) {
      const code = text.codePointAt(i) ?? 0;
      const units = code > 0xffff ? 2 : 1;
      const width = code < 0x80 ? 1 : code < 0x800 ? 2 : code < 0x10000 ? 3 : 4;
      for (let unit = 0; unit < units; unit++) bytes[i + unit] = total;
      total += width;
      i += units;
    }
    bytes[text.length] = total;
    this.byteAt = bytes;
  }

  /** The UTF-16 offset of the character that starts at `byte` (rounded down inside a character). */
  toIndex(byte: number): number {
    const bytes = this.byteAt;
    let low = 0;
    let high = bytes.length - 1;
    while (low < high) {
      const mid = (low + high + 1) >>> 1;
      if ((bytes[mid] ?? 0) <= byte) low = mid;
      else high = mid - 1;
    }
    while (low > 0 && bytes[low - 1] === bytes[low]) low -= 1;
    return low;
  }

  toByte(index: number): number {
    return this.byteAt[Math.max(0, Math.min(index, this.text.length))] ?? 0;
  }
}
