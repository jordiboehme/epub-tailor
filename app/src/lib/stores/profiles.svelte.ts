// The device profiles the picker offers, loaded once from the CLI. The
// built-in list comes from `profiles --report json`; when the user has layered
// their own profile JSON on top, the composed appendix (which decides a
// self-overwriting output's ".<appendix>.epub" suffix) is resolved by asking
// the CLI to compose the specs, because last-layer-wins is the CLI's rule to
// apply, not ours to guess.

import { runSidecar } from "../api/sidecar";
import { isCliFailure, parseReport } from "../api/contract";
import type { Profile, ProfilesReport } from "../api/contract";
import { settings } from "./settings.svelte";
import type { ProfileLayer } from "./settings.svelte";

const FALLBACK_APPENDIX = "tailored";

/**
 * A file layer's display name: the last path segment, either separator.
 * Exported so `ProfilePicker` renders the same label it stamps into
 * `stackLabel()`, instead of keeping a second copy that could drift.
 */
export function baseName(path: string): string {
  return path.split(/[/\\]/).pop() || path;
}

/**
 * Append a built-in layer, unless it is already in the stack - a duplicate
 * built-in is meaningless (it would just shadow itself) and would corrupt
 * `stackLabel()`'s output. Returns the same array reference when it is a
 * no-op, so a caller can tell nothing changed.
 */
export function addBuiltinLayer(stack: ProfileLayer[], name: string): ProfileLayer[] {
  if (stack.some((layer) => layer.kind === "builtin" && layer.name === name)) return stack;
  return [...stack, { kind: "builtin", name }];
}

/** Append a file layer, unless the same path is already in the stack. */
export function addFileLayer(stack: ProfileLayer[], path: string): ProfileLayer[] {
  if (stack.some((layer) => layer.kind === "file" && layer.path === path)) return stack;
  return [...stack, { kind: "file", path }];
}

/**
 * Why `addFileLayer` or `replaceLayerAt` returned the stack unchanged, phrased
 * for the user, or `null` when the path is genuinely new. The built-in panel
 * can show this state up front (it disables an already-present entry and
 * relabels it), but the file picker cannot: the app only learns the path after
 * the OS dialog closes, so a duplicate there reads as a click the app ignored
 * unless it says otherwise.
 *
 * `ignoreIndex` is the layer being replaced, which must not count as its own
 * duplicate - re-picking the file a layer already holds is a no-op, not a
 * clash, and saying otherwise would read as a refusal.
 */
export function describeDuplicateLayer(
  stack: ProfileLayer[],
  path: string,
  ignoreIndex?: number,
): string | null {
  const clash = stack.some(
    (layer, i) => i !== ignoreIndex && layer.kind === "file" && layer.path === path,
  );
  return clash ? `${baseName(path)} is already in the stack` : null;
}

/**
 * Remove the layer at `index`, refusing when it is the only one left - a
 * stack of zero layers is never a valid composition (see
 * `migrateProfileStack`'s docstring in `settings.svelte.ts`). Returns the
 * same array reference on refusal.
 */
export function removeLayerAt(stack: ProfileLayer[], index: number): ProfileLayer[] {
  if (stack.length <= 1) return stack;
  return stack.filter((_, i) => i !== index);
}

/**
 * A layer's identity: what makes two layers the same layer. Exported so
 * `ProfilePicker`'s keyed `{#each}` and the duplicate guards below agree on
 * one definition instead of each carrying its own.
 */
export function layerKey(layer: ProfileLayer): string {
  return layer.kind === "builtin" ? `builtin:${layer.name}` : `file:${layer.path}`;
}

/**
 * Swap the layer at `index` for `next`, which is the only way to change a
 * layer that cannot be moved or removed - a stack of one, where every other
 * control is necessarily disabled. Refuses (returning the same array
 * reference) when `index` is out of range, when `next` is already the layer
 * there, or when it duplicates a layer elsewhere in the stack: two layers
 * sharing a `layerKey` throw Svelte's `each_key_duplicate` at render, the same
 * hazard `addBuiltinLayer` and `addFileLayer` guard against.
 */
export function replaceLayerAt(
  stack: ProfileLayer[],
  index: number,
  next: ProfileLayer,
): ProfileLayer[] {
  if (index < 0 || index >= stack.length) return stack;
  const key = layerKey(next);
  if (stack.some((layer, i) => i !== index && layerKey(layer) === key)) return stack;
  if (layerKey(stack[index]) === key) return stack;
  return stack.map((layer, i) => (i === index ? next : layer));
}

/**
 * Swap the layer at `index` with its neighbour `delta` away (-1 to move up,
 * +1 to move down). A move past either end is a no-op, returning the same
 * array reference.
 */
export function moveLayer(stack: ProfileLayer[], index: number, delta: number): ProfileLayer[] {
  const target = index + delta;
  if (target < 0 || target >= stack.length) return stack;
  const next = [...stack];
  [next[index], next[target]] = [next[target], next[index]];
  return next;
}

/**
 * The sentinel `DeviceCaps::permissive()` (Rust) fills `screen_w`/`screen_h`
 * with for a profile that carries no device screen at all: `u32::MAX`, not
 * `0` - `generic` and `epub` both report `screen_w: 4294967295` from the real
 * CLI. `0` is also treated as "no screen", as a defensive fallback for a
 * malformed profile, but it is not the value either built-in actually uses.
 */
const NO_SCREEN_SENTINEL = 4294967295;

/** Whether `caps` describes a real device screen, as opposed to the
 * device-neutral sentinel `generic` and `epub` both report. */
export function hasScreen(caps: { screen_w: number; screen_h: number }): boolean {
  return (
    caps.screen_w > 0 &&
    caps.screen_w < NO_SCREEN_SENTINEL &&
    caps.screen_h > 0 &&
    caps.screen_h < NO_SCREEN_SENTINEL
  );
}

/**
 * True when more than one layer carries a device screen, which means the
 * CLI's last-layer-wins composition silently discards all but the last
 * screen. `generic` and `epub` report the device-neutral sentinel (see
 * `hasScreen`) and never count, so pairing either with one device profile -
 * including the flagship `[epub, generic, x4]` stack - does not warn.
 */
export function hasDeviceClash(stack: ProfileLayer[], builtins: Profile[]): boolean {
  return (
    stack.filter((layer) => {
      if (layer.kind !== "builtin") return false;
      const caps = builtins.find((p) => p.name === layer.name)?.caps;
      return caps !== undefined && hasScreen(caps);
    }).length > 1
  );
}

/** The `profiles <specs> --report json` payload: one resolved composition. */
interface ResolvedProfileReport {
  schema: 1;
  profile: Profile;
}

class ProfilesStore {
  /** The built-in profiles, for the picker. */
  builtins = $state<Profile[]>([]);
  /** True once the built-in list has loaded. */
  ready = $state(false);

  /** Load the built-in profile list once at startup. */
  async load(): Promise<void> {
    const result = await runSidecar(["profiles", "--report", "json"]);
    const report = parseReport<ProfilesReport>(result.stdout, "profiles");
    if (!isCliFailure(report)) {
      this.builtins = report.profiles;
    }
    this.ready = true;
  }

  /** Each layer as a CLI spec: a built-in name, or a path to a JSON file. */
  activeProfileSpecs(): string[] {
    return settings.profileStack.map((layer) => (layer.kind === "builtin" ? layer.name : layer.path));
  }

  /**
   * The stack as one label, for the output stamp and the copies index. A
   * single-element stack renders as a bare name - not `name+` - so files
   * fitted before stacking existed still match on their profile name and no
   * rerun churn occurs. A `file` layer contributes its basename, not the full
   * path: the label is for display and stamping, not round-tripping a spec.
   */
  stackLabel(): string {
    return settings.profileStack
      .map((layer) => (layer.kind === "builtin" ? layer.name : baseName(layer.path)))
      .join("+");
  }

  /**
   * The appendix the active composition stamps onto a self-overwriting output.
   * Always resolved through the CLI's own composition: even a stack of only
   * built-ins (e.g. `x4` + `generic`) is not simply the first layer's
   * appendix, since a later layer can override it - last-layer-wins is the
   * CLI's rule to apply, not ours to guess from the loaded built-in list.
   */
  async activeAppendix(): Promise<string> {
    const result = await runSidecar(["profiles", ...this.activeProfileSpecs(), "--report", "json"]);
    try {
      const report = parseReport<ResolvedProfileReport>(result.stdout, "profiles");
      if (!isCliFailure(report)) {
        return report.profile.appendix ?? FALLBACK_APPENDIX;
      }
    } catch {
      // A composition that fails to resolve (e.g. a bad user path) should not
      // block a run; fall back to the default appendix and let the conversion
      // itself surface the real error.
    }
    return FALLBACK_APPENDIX;
  }
}

export const profiles = new ProfilesStore();
