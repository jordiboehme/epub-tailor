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

const FALLBACK_APPENDIX = "tailored";

/** A file layer's display name: the last path segment, either separator. */
function baseName(path: string): string {
  return path.split(/[/\\]/).pop() || path;
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
