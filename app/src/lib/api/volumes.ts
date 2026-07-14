// Removable volume listing, for "eject when done" style workflows: point a
// job's output at a plugged-in reader and get a live list of what is
// available to write to.

import { invoke } from "@tauri-apps/api/core";

export interface Volume {
  name: string;
  path: string;
}

export async function listRemovableVolumes(): Promise<Volume[]> {
  return invoke<Volume[]>("list_removable_volumes");
}

const DEFAULT_POLL_INTERVAL_MS = 2000;

/**
 * Poll `list_removable_volumes` every `intervalMs` and call `onChange` when
 * the list actually changes (a drive was plugged in or removed). Returns a
 * `stop()` that cancels the timer; a tick already in flight when `stop()` is
 * called is still allowed to finish but will not call `onChange`.
 */
export function pollVolumes(
  onChange: (volumes: Volume[]) => void,
  intervalMs: number = DEFAULT_POLL_INTERVAL_MS,
): () => void {
  let stopped = false;
  let lastSnapshot = "";

  const tick = async () => {
    let volumes: Volume[];
    try {
      volumes = await listRemovableVolumes();
    } catch {
      // A transient invoke failure should not stop polling; just try again
      // on the next tick.
      return;
    }
    if (stopped) return;
    const snapshot = JSON.stringify(volumes);
    if (snapshot !== lastSnapshot) {
      lastSnapshot = snapshot;
      onChange(volumes);
    }
  };

  void tick();
  const id = setInterval(() => void tick(), intervalMs);

  return () => {
    stopped = true;
    clearInterval(id);
  };
}
