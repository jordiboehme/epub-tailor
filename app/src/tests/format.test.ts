import { describe, expect, it } from "vitest";
import { formatSize } from "../lib/api/format";

describe("formatSize", () => {
  it("uses bytes below a kilobyte", () => {
    expect(formatSize(0)).toBe("0 B");
    expect(formatSize(512)).toBe("512 B");
    expect(formatSize(1023)).toBe("1023 B");
  });

  it("uses kilobytes for a small book, so 2 KB never reads as 0.0 MB", () => {
    expect(formatSize(2048)).toBe("2.0 KB");
    expect(formatSize(1536)).toBe("1.5 KB");
  });

  it("drops the decimal once the number is big enough to carry itself", () => {
    expect(formatSize(20 * 1024)).toBe("20 KB");
    expect(formatSize(999 * 1024)).toBe("999 KB");
  });

  it("switches to megabytes at a thousand kilobytes, never printing 1024 KB", () => {
    expect(formatSize(1000 * 1024)).toBe("1.0 MB");
    expect(formatSize(1024 * 1024)).toBe("1.0 MB");
    expect(formatSize(5 * 1024 * 1024)).toBe("5.0 MB");
    expect(formatSize(12.34 * 1024 * 1024)).toBe("12.3 MB");
  });

  it("treats a negative or non-finite size as nothing rather than printing NaN", () => {
    expect(formatSize(-1)).toBe("0 B");
    expect(formatSize(Number.NaN)).toBe("0 B");
  });
});
