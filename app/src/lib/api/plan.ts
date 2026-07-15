// The two-pass output planner, resolving batch and on-disk collisions - the
// one planning path every Fit-mode run goes through. Uses the `paths_exist`
// command, so it lives with covers.ts among the api/ modules that legitimately
// touch Tauri (the pure name math it wraps is planOutputs, tested on its own
// in outputs.test.ts).

import { invoke } from "@tauri-apps/api/core";
import { planOutputs } from "./outputs";
import type { OutputPlan, PlanOptions, PlannedBook } from "./outputs";

/** Everything planOutputs needs except the disk probe, which this module supplies. */
export type PlanSettings = Omit<PlanOptions, "existsOnDisk">;

/**
 * Plan an output path for each book. Plan once ignoring disk to learn the
 * candidate paths, ask the OS which already exist, then plan again so real
 * collisions get numbered.
 */
export async function resolvePlans(
  planned: PlannedBook[],
  opts: PlanSettings,
): Promise<OutputPlan[]> {
  const draft = planOutputs(planned, { ...opts, existsOnDisk: () => false });
  const candidates = draft.map((p) => p.output);

  const existing = new Set<string>();
  if (candidates.length > 0) {
    const flags = await invoke<boolean[]>("paths_exist", { paths: candidates });
    candidates.forEach((path, i) => {
      if (flags[i]) existing.add(path);
    });
  }

  return planOutputs(planned, { ...opts, existsOnDisk: (p) => existing.has(p) });
}
