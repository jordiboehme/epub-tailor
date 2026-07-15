import { describe, expect, it } from "vitest";
import { planOutputs, previewOutputName } from "../lib/api/outputs";
import type { PlannedBook } from "../lib/api/outputs";

const epub = (input: string, template: Partial<PlannedBook["template"]> = {}): PlannedBook => ({
  input,
  kind: "epub",
  template: {
    originalStem: input
      .slice(input.lastIndexOf("/") + 1)
      .replace(/\.[^.]+$/, ""),
    ...template,
  },
});

const md = (input: string, template: Partial<PlannedBook["template"]> = {}): PlannedBook => ({
  ...epub(input, template),
  kind: "md",
});

const noDisk = () => false;

describe("planOutputs", () => {
  it("writes alongside each original by default", () => {
    const plans = planOutputs([epub("/books/Dune.epub", { title: "Dune", authors: ["Herbert"] })], {
      template: "{author} - {title}",
      outputDir: null,
      appendix: "tailored",
      existsOnDisk: noDisk,
    });
    expect(plans).toEqual([{ input: "/books/Dune.epub", output: "/books/Herbert - Dune.epub" }]);
  });

  it("writes into an explicit output directory when given one", () => {
    const plans = planOutputs([epub("/books/Dune.epub", { title: "Dune", authors: ["Herbert"] })], {
      template: "{author} - {title}",
      outputDir: "/out",
      appendix: "tailored",
      existsOnDisk: noDisk,
    });
    expect(plans[0].output).toBe("/out/Herbert - Dune.epub");
  });

  it("numbers a batch-internal name collision, keeping the first plain", () => {
    const plans = planOutputs(
      [
        epub("/a/one.epub", { title: "Same" }),
        epub("/b/two.epub", { title: "Same" }),
      ],
      {
        template: "{title}",
        outputDir: "/out",
        appendix: "tailored",
        existsOnDisk: noDisk,
      },
    );
    expect(plans.map((p) => p.output)).toEqual(["/out/Same.epub", "/out/Same (2).epub"]);
  });

  it("does not collide identical names that land in different directories", () => {
    const plans = planOutputs(
      [
        epub("/a/one.epub", { title: "Same" }),
        epub("/b/two.epub", { title: "Same" }),
      ],
      {
        template: "{title}",
        outputDir: null,
        appendix: "tailored",
        existsOnDisk: noDisk,
      },
    );
    expect(plans.map((p) => p.output)).toEqual(["/a/Same.epub", "/b/Same.epub"]);
  });

  it("inserts the appendix when the output would overwrite its own input", () => {
    const plans = planOutputs([epub("/books/Dune.epub")], {
      template: "{original}",
      outputDir: null,
      appendix: "tailored",
      existsOnDisk: noDisk,
    });
    expect(plans[0].output).toBe("/books/Dune.tailored.epub");
  });

  it("inserts the appendix case-insensitively against the input extension", () => {
    const plans = planOutputs([epub("/books/Dune.EPUB")], {
      template: "{original}",
      outputDir: null,
      appendix: "x4",
      existsOnDisk: noDisk,
    });
    expect(plans[0].output).toBe("/books/Dune.x4.epub");
  });

  it("keeps a double-hyphen stem intact and still inserts the appendix", () => {
    const plans = planOutputs([epub("/books/Grays Sports Almanac -- 1950-2000.epub")], {
      template: "{original}",
      outputDir: null,
      appendix: "x4",
      existsOnDisk: noDisk,
    });
    expect(plans[0].output).toBe("/books/Grays Sports Almanac -- 1950-2000.x4.epub");
  });

  it("inserts the appendix even when sanitization had to alter the stem", () => {
    // A ":" can live in a stem synced in from another OS; the rendered name
    // differs from the input then, but it is still the book's own name, so
    // writing it without the appendix would just be a silent rename.
    const plans = planOutputs([epub("/books/A:B.epub")], {
      template: "{original}",
      outputDir: null,
      appendix: "x4",
      existsOnDisk: noDisk,
    });
    expect(plans[0].output).toBe("/books/A-B.x4.epub");
  });

  it("never plans an in-place write - every plan carries a real path", () => {
    // In-place runs are Edit mode's business and never come out of the
    // planner: Fit mode always produces a copy.
    const plans = planOutputs([epub("/b/A.epub"), md("/b/notes.md")], {
      template: "{original}",
      outputDir: null,
      appendix: "tailored",
      existsOnDisk: noDisk,
    });
    expect(plans.every((p) => typeof p.output === "string")).toBe(true);
  });

  it("numbers a collision against a file already on disk", () => {
    const plans = planOutputs([epub("/books/Dune.epub", { title: "Foo" })], {
      template: "{title}",
      outputDir: "/out",
      appendix: "tailored",
      existsOnDisk: (p) => p === "/out/Foo.epub",
    });
    expect(plans[0].output).toBe("/out/Foo (2).epub");
  });

  it("always gives an md book an .epub output", () => {
    const plans = planOutputs([md("/docs/guide.md", { title: "Guide" })], {
      template: "{title}",
      outputDir: null,
      appendix: "tailored",
      existsOnDisk: noDisk,
    });
    expect(plans[0].output).toBe("/docs/Guide.epub");
  });

  it("treats a backslash as a path separator when splitting the input directory", () => {
    const plans = planOutputs(
      [{ input: "C:\\Books\\Dune.epub", kind: "epub", template: { originalStem: "Dune" } }],
      {
        template: "{original}",
        outputDir: null,
        appendix: "tailored",
        existsOnDisk: noDisk,
      },
    );
    // Output joins with '/', but the directory comes from the backslash split.
    expect(plans[0].output).toBe("C:\\Books/Dune.tailored.epub");
  });
});

describe("previewOutputName", () => {
  const opts = {
    template: "{original}",
    outputDir: null,
    appendix: "tailored",
  };

  it("shows the appendix the planner inserts in the default configuration", () => {
    // {original} + "Alongside originals": every output lands on its own input,
    // so what actually gets written is Dune.tailored.epub - and the preview has
    // to say so, or it lies to every user on first launch.
    expect(previewOutputName(epub("/books/Dune.epub"), opts)).toBe("Dune.tailored.epub");
  });

  it("shows the plain name when the template moves the output off its input", () => {
    const book = epub("/books/Dune.epub", { title: "Dune", authors: ["Herbert"] });
    expect(previewOutputName(book, { ...opts, template: "{author} - {title}" })).toBe(
      "Herbert - Dune.epub",
    );
  });

  it("shows the plain name when the output goes to another folder", () => {
    expect(previewOutputName(epub("/books/Dune.epub"), { ...opts, outputDir: "/out" })).toBe(
      "Dune.epub",
    );
  });

  it("is just the file name, never the whole path", () => {
    const name = previewOutputName(epub("/books/Dune.epub"), { ...opts, outputDir: "/out/box" });
    expect(name).toBe("Dune.epub");
  });
});
