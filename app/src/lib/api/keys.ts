// The workbench's keyboard shortcuts, as a pure function of the chord. The
// window listener in Workbench.svelte reads the DOM event, this decides what
// it means - so the rules (including "never steal a key from a text field")
// are unit-testable without a window.

/** A keydown, reduced to the parts a shortcut decision actually depends on. */
export interface KeyChord {
  key: string;
  metaKey: boolean;
  ctrlKey: boolean;
  /** Focus is in an input, textarea or contenteditable: the keys are the field's. */
  inTextField: boolean;
  /** A modal is up: its own Escape (and everything else) takes precedence. */
  modalOpen?: boolean;
}

export type Shortcut = "select-all" | "clear-selection" | "remove-selected";

/**
 * The shortcut a chord means, or `null` for the many that mean nothing.
 * Nothing fires while a text field has focus: Cmd-A there selects the text,
 * Backspace deletes a character, and Escape is the field's business - stealing
 * any of the three would be a bug, not a feature. Nor while a modal is up: its
 * Escape closes it, and the workbench underneath should not also lose its
 * selection on the way out.
 */
export function shortcutFor(chord: KeyChord): Shortcut | null {
  if (chord.inTextField || chord.modalOpen) return null;

  const key = chord.key;
  if ((chord.metaKey || chord.ctrlKey) && key.toLowerCase() === "a") return "select-all";
  if (chord.metaKey || chord.ctrlKey) return null;
  if (key === "Escape") return "clear-selection";
  if (key === "Delete" || key === "Backspace") return "remove-selected";
  return null;
}

/** Whether an event target is a field whose keystrokes are its own. */
export function isTextField(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  const tag = target.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT";
}

/** Whether a modal dialog is currently mounted (both of ours are `aria-modal`). */
export function isModalOpen(): boolean {
  return document.querySelector('[role="dialog"][aria-modal="true"]') !== null;
}
