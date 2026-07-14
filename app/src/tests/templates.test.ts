import { describe, expect, it } from "vitest";
import { renderTemplate, resolveCollisions } from "../lib/api/templates";
import type { TemplateBook } from "../lib/api/templates";

const book = (overrides: Partial<TemplateBook> = {}): TemplateBook => ({
  title: "The Fellowship of the Ring",
  authors: ["J.R.R. Tolkien"],
  series: "The Lord of the Rings",
  seriesIndex: "1",
  originalStem: "lotr1",
  ...overrides,
});

describe("renderTemplate", () => {
  it("substitutes the happy path", () => {
    expect(renderTemplate("{author} - {title}", book())).toBe(
      "J.R.R. Tolkien - The Fellowship of the Ring",
    );
  });

  it("substitutes every token", () => {
    const b = book({ authors: ["Jane Author", "Bill Writer"] });
    const result = renderTemplate(
      "{title}_{author}_{authors}_{series}_{series_index}_{original}",
      b,
    );
    expect(result).toBe(
      "The Fellowship of the Ring_Jane Author_Jane Author & Bill Writer_The Lord of the Rings_1_lotr1",
    );
  });

  it("renders an unknown token literally so typos stay visible", () => {
    expect(renderTemplate("{title} {oops}", book())).toBe(
      "The Fellowship of the Ring {oops}",
    );
  });

  it("sanitizes hostile characters in the substituted values", () => {
    const b = book({ title: "a/b: c?*", authors: [] });
    // "/", ":", "?", "*" all become "-"; runs of "-" and spaces collapse.
    expect(renderTemplate("{title}", b)).toBe("a-b- c-");
  });

  it("falls back to the original stem when sanitization empties the result", () => {
    // Trailing dots are stripped (the Windows rule); an all-dots title has
    // nothing left afterwards.
    const b = book({ title: "...", authors: [] });
    expect(renderTemplate("{title}", b)).toBe(b.originalStem);
  });

  it("caps the rendered name at 120 characters", () => {
    const b = book({ title: "x".repeat(200), authors: [] });
    const result = renderTemplate("{title}", b);
    expect(result.length).toBeLessThanOrEqual(120);
    expect(result).toBe("x".repeat(120));
  });

  it("renders a missing token as an empty string", () => {
    const b = book({ title: "Solo", authors: [], series: undefined });
    expect(renderTemplate("{title}{author}", b)).toBe("Solo");
  });
});

describe("resolveCollisions", () => {
  it("leaves unique names untouched", () => {
    const result = resolveCollisions(["a", "b", "c"], {
      existsOnDisk: () => false,
    });
    expect(result).toEqual(["a", "b", "c"]);
  });

  it("numbers batch-internal collisions, keeping the first occurrence plain", () => {
    const result = resolveCollisions(["book", "book", "book"], {
      existsOnDisk: () => false,
    });
    expect(result).toEqual(["book", "book (2)", "book (3)"]);
  });

  it("numbers collisions against files already on disk", () => {
    const result = resolveCollisions(["book"], {
      existsOnDisk: (name) => name === "book",
    });
    expect(result).toEqual(["book (2)"]);
  });

  it("treats collisions case-insensitively", () => {
    const result = resolveCollisions(["Book", "book", "BOOK"], {
      existsOnDisk: () => false,
    });
    expect(result).toEqual(["Book", "book (2)", "BOOK (3)"]);
  });
});
