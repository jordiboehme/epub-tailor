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
  return env ? { env } : undefined;
}

/** Run the sidecar once, waiting for it to exit. For commands that print one document and stop. */
export async function runSidecar(
  argv: string[],
  opts?: { env?: Record<string, string> },
): Promise<SidecarResult> {
  const command = Command.sidecar(SIDECAR, argv, spawnOptions(opts?.env));
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
    const command = Command.sidecar(SIDECAR, argv, spawnOptions(opts?.env));

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
