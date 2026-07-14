// Runs the bundled `epub-tailor` CLI as a Tauri sidecar. Two shapes: a
// one-shot run for a single call (`profiles --report json`, `metadata show`)
// and a spawn-and-stream run for a conversion, where the caller wants stderr
// lines as they arrive to show live progress.

import { Command } from "@tauri-apps/plugin-shell";
import type { SpawnOptions } from "@tauri-apps/plugin-shell";

const SIDECAR = "binaries/epub-tailor";

/** How many trailing stderr lines a spawned run keeps in its final result. */
const STDERR_TAIL_LINES = 50;

export interface SidecarResult {
  code: number | null;
  stdout: string;
  stderr: string;
}

function spawnOptions(env?: Record<string, string>): SpawnOptions | undefined {
  // Copied for the same reason as the argv copies below: `env` may one day be
  // read out of a Svelte $state store too, and a proxy must never reach here.
  return env ? { env: { ...env } } : undefined;
}

/** Run the sidecar once, waiting for it to exit. For commands that print one document and stop. */
export async function runSidecar(
  argv: string[],
  opts?: { env?: Record<string, string> },
): Promise<SidecarResult> {
  // Callers (e.g. the job store) may hand us a `job.argv` that lives inside a
  // Svelte 5 `$state` array - Svelte deep-proxies it, and passing that proxy
  // straight into `Command.sidecar` makes `spawn()`/`execute()` reject with
  // Svelte's "state_descriptors_fixed" proxy-trap error. Copy into a fresh
  // plain array here, at the choke point, so a proxy never crosses into the
  // Tauri IPC layer (elements are primitives, so a shallow copy fully de-proxies).
  const args = [...argv];
  const command = Command.sidecar(SIDECAR, args, spawnOptions(opts?.env));
  const output = await command.execute();
  return { code: output.code, stdout: output.stdout, stderr: output.stderr };
}

export interface SidecarHandle {
  kill(): Promise<void>;
  done: Promise<SidecarResult>;
}

/**
 * Spawn the sidecar and stream it: stdout is accumulated line by line (the
 * shell plugin delivers it that way; the JSON document arrives complete by
 * the time the process closes), stderr lines are handed to `onStderrLine` as
 * they arrive for a live progress view and the last `STDERR_TAIL_LINES`
 * are kept in the resolved result. `done` always resolves - even if the
 * process itself never started - so a caller awaiting it can never hang.
 */
export function spawnSidecar(
  argv: string[],
  opts?: { env?: Record<string, string>; onStderrLine?: (line: string) => void },
): Promise<SidecarHandle> {
  return new Promise((resolve, reject) => {
    // See runSidecar: de-proxy argv before it reaches Command.sidecar.
    const args = [...argv];
    const command = Command.sidecar(SIDECAR, args, spawnOptions(opts?.env));

    const stdoutLines: string[] = [];
    const stderrTail: string[] = [];
    let settleDone: (result: SidecarResult) => void;
    const done = new Promise<SidecarResult>((res) => {
      settleDone = res;
    });

    const finish = (code: number | null, errorLine?: string) => {
      if (errorLine !== undefined) {
        stderrTail.push(errorLine);
      }
      const tail = stderrTail.slice(-STDERR_TAIL_LINES);
      settleDone({ code, stdout: stdoutLines.join("\n"), stderr: tail.join("\n") });
    };

    command.stdout.on("data", (line) => {
      stdoutLines.push(line);
    });
    command.stderr.on("data", (line) => {
      stderrTail.push(line);
      opts?.onStderrLine?.(line);
    });
    command.on("close", (payload) => finish(payload.code));
    command.on("error", (error) => finish(null, String(error)));

    command
      .spawn()
      .then((child) => {
        resolve({
          kill: () => child.kill(),
          done,
        });
      })
      .catch(reject);
  });
}
