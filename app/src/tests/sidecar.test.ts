import { describe, expect, it, vi, beforeEach } from "vitest";

// Each recorded call is [program, args, options] as handed to `Command.sidecar`.
const sidecarCalls: unknown[][] = [];

vi.mock("@tauri-apps/plugin-shell", () => {
  return {
    Command: {
      sidecar: vi.fn((program: string, args: unknown, options: unknown) => {
        // Mirrors the real IPC boundary touching property descriptors while
        // marshalling argv: harmless on a plain array, but it reproduces the
        // Svelte "state_descriptors_fixed" throw when `args` is still a
        // Svelte $state proxy whose `defineProperty` trap rejects redefines.
        if (Array.isArray(args)) {
          args.forEach((value, i) => {
            Object.defineProperty(args, i, { value, enumerable: true, writable: true, configurable: true });
          });
        }
        sidecarCalls.push([program, args, options]);
        return {
          execute: vi.fn().mockResolvedValue({ code: 0, stdout: "", stderr: "" }),
          stdout: { on: vi.fn() },
          stderr: { on: vi.fn() },
          on: vi.fn(),
          spawn: vi.fn().mockResolvedValue({ kill: vi.fn() }),
        };
      }),
    },
  };
});

import { runSidecar, spawnSidecar } from "../lib/api/sidecar";

/** Wraps `arr` so redefining any property throws, mimicking a Svelte 5 `$state` proxy. */
function stateProxyLike<T extends object>(arr: T): T {
  return new Proxy(arr, {
    defineProperty() {
      throw new Error("Svelte error: state_descriptors_fixed");
    },
  });
}

beforeEach(() => {
  sidecarCalls.length = 0;
});

describe("runSidecar", () => {
  it("hands Command.sidecar a fresh array, not the original reference", async () => {
    const argv = ["metadata", "show", "/book.epub"];
    await runSidecar(argv);
    expect(sidecarCalls).toHaveLength(1);
    const [, receivedArgs] = sidecarCalls[0];
    expect(receivedArgs).not.toBe(argv);
    expect(receivedArgs).toEqual(argv);
  });

  it("survives a Svelte-state-proxy argv without throwing, delivering a plain equal array", async () => {
    const argv = stateProxyLike(["metadata", "show", "/book.epub"]);
    await expect(runSidecar(argv)).resolves.toEqual({ code: 0, stdout: "", stderr: "" });
    const [, receivedArgs] = sidecarCalls[0];
    expect(Array.isArray(receivedArgs)).toBe(true);
    expect(receivedArgs).not.toBe(argv);
    expect(receivedArgs).toEqual(["metadata", "show", "/book.epub"]);
  });
});

describe("spawnSidecar", () => {
  it("hands Command.sidecar a fresh array, not the original reference", async () => {
    const argv = ["fit", "/book.epub", "--report", "json"];
    await spawnSidecar(argv);
    expect(sidecarCalls).toHaveLength(1);
    const [, receivedArgs] = sidecarCalls[0];
    expect(receivedArgs).not.toBe(argv);
    expect(receivedArgs).toEqual(argv);
  });

  it("survives a Svelte-state-proxy argv without throwing, delivering a plain equal array", async () => {
    const argv = stateProxyLike(["fit", "/book.epub", "--report", "json"]);
    await expect(spawnSidecar(argv)).resolves.toBeDefined();
    const [, receivedArgs] = sidecarCalls[0];
    expect(Array.isArray(receivedArgs)).toBe(true);
    expect(receivedArgs).not.toBe(argv);
    expect(receivedArgs).toEqual(["fit", "/book.epub", "--report", "json"]);
  });
});
