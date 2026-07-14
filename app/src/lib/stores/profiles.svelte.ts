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

  /** The selected built-in name followed by any user JSON layers, composed left to right. */
  activeProfileSpecs(): string[] {
    return [settings.profile, ...settings.userProfilePaths];
  }

  /**
   * The appendix the active composition stamps onto a self-overwriting output.
   * With no user layer this is just the selected built-in's appendix (from the
   * already-loaded list); with a layer it is whatever the composed profile
   * resolves to, so the CLI does the last-wins reasoning.
   */
  async activeAppendix(): Promise<string> {
    if (settings.userProfilePaths.length === 0) {
      const builtin = this.builtins.find((p) => p.name === settings.profile);
      return builtin?.appendix ?? FALLBACK_APPENDIX;
    }
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
