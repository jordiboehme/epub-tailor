import { describe, expect, it } from "vitest";
import { closePrompt } from "../lib/api/close-guard";

describe("closePrompt", () => {
  it("stays silent when the workbench is empty, whatever else is going on", () => {
    expect(closePrompt({ books: 0, editedFiles: 0, jobsRunning: false })).toBeNull();
    expect(closePrompt({ books: 0, editedFiles: 3, jobsRunning: true })).toBeNull();
  });

  it("names the book count, singular and plural", () => {
    const one = closePrompt({ books: 1, editedFiles: 0, jobsRunning: false });
    expect(one?.message).toContain("1 book on the workbench");
    expect(one?.message).not.toContain("books on the workbench");

    const many = closePrompt({ books: 4, editedFiles: 0, jobsRunning: false });
    expect(many?.message).toContain("4 books on the workbench");
  });

  it("mentions staged edits only when there are any", () => {
    const none = closePrompt({ books: 2, editedFiles: 0, jobsRunning: false });
    expect(none?.message).not.toContain("metadata edits");

    const one = closePrompt({ books: 2, editedFiles: 1, jobsRunning: false });
    expect(one?.message).toContain("1 file has staged metadata edits");

    const many = closePrompt({ books: 2, editedFiles: 3, jobsRunning: false });
    expect(many?.message).toContain("3 files have staged metadata edits");
  });

  it("mentions the running conversion only while one runs", () => {
    const idle = closePrompt({ books: 2, editedFiles: 0, jobsRunning: false });
    expect(idle?.message).not.toContain("conversion");

    const busy = closePrompt({ books: 2, editedFiles: 0, jobsRunning: true });
    expect(busy?.message).toContain("A conversion is still running and will be stopped.");
  });

  it("keeps a stable title", () => {
    const prompt = closePrompt({ books: 1, editedFiles: 1, jobsRunning: true });
    expect(prompt?.title).toBe("Close EPUB Tailor?");
  });
});
