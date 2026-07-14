import { describe, expect, it } from "vitest";
import { shortcutFor } from "../lib/api/keys";
import type { KeyChord } from "../lib/api/keys";

const chord = (overrides: Partial<KeyChord> = {}): KeyChord => ({
  key: "a",
  metaKey: false,
  ctrlKey: false,
  inTextField: false,
  ...overrides,
});

describe("shortcutFor", () => {
  it("reads Cmd-A and Ctrl-A as select all", () => {
    expect(shortcutFor(chord({ key: "a", metaKey: true }))).toBe("select-all");
    expect(shortcutFor(chord({ key: "a", ctrlKey: true }))).toBe("select-all");
    expect(shortcutFor(chord({ key: "A", metaKey: true }))).toBe("select-all");
  });

  it("ignores a bare A: that is just typing", () => {
    expect(shortcutFor(chord({ key: "a" }))).toBeNull();
  });

  it("reads Escape as clear selection", () => {
    expect(shortcutFor(chord({ key: "Escape" }))).toBe("clear-selection");
  });

  it("reads Delete and Backspace as remove selected", () => {
    expect(shortcutFor(chord({ key: "Delete" }))).toBe("remove-selected");
    expect(shortcutFor(chord({ key: "Backspace" }))).toBe("remove-selected");
  });

  it("stays out of the way entirely while a text field has focus", () => {
    expect(shortcutFor(chord({ key: "a", metaKey: true, inTextField: true }))).toBeNull();
    expect(shortcutFor(chord({ key: "Backspace", inTextField: true }))).toBeNull();
    expect(shortcutFor(chord({ key: "Delete", inTextField: true }))).toBeNull();
    expect(shortcutFor(chord({ key: "Escape", inTextField: true }))).toBeNull();
  });

  it("stays out of the way while a modal is up: its Escape is the dialog's", () => {
    expect(shortcutFor(chord({ key: "Escape", modalOpen: true }))).toBeNull();
    expect(shortcutFor(chord({ key: "a", metaKey: true, modalOpen: true }))).toBeNull();
    expect(shortcutFor(chord({ key: "Delete", modalOpen: true }))).toBeNull();
  });

  it("has no opinion about any other key", () => {
    expect(shortcutFor(chord({ key: "Enter" }))).toBeNull();
    expect(shortcutFor(chord({ key: "s", metaKey: true }))).toBeNull();
  });
});
