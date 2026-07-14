#!/usr/bin/env node
// Builds the epub-tailor CLI in release mode and copies it into
// src-tauri/binaries/ under Tauri's sidecar naming convention
// (epub-tailor-<target-triple>[.exe]). Tauri's `externalBin` resolves the
// per-platform binary by that suffix, so `beforeDevCommand`/`beforeBuildCommand`
// run this first. All paths resolve from the script location, never cwd.

import { execFileSync } from "node:child_process";
import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  readdirSync,
} from "node:fs";
import { homedir, platform } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const appDir = resolve(scriptDir, "..");
const repoRoot = resolve(appDir, "..");
const exe = platform() === "win32" ? ".exe" : "";

// Locate cargo: PATH first (the common case), then one sensible rustup/cargo
// fallback for shells that do not put ~/.cargo/bin on PATH, then $CARGO.
function locateCargo() {
  if (process.env.CARGO) return process.env.CARGO;
  try {
    execFileSync("cargo", ["--version"], { stdio: "ignore" });
    return "cargo";
  } catch {
    // fall through to the fallbacks below
  }
  const candidates = [join(homedir(), ".cargo", "bin", `cargo${exe}`)];
  const toolchains = join(homedir(), ".rustup", "toolchains");
  if (existsSync(toolchains)) {
    for (const tc of readdirSync(toolchains)) {
      candidates.push(join(toolchains, tc, "bin", `cargo${exe}`));
    }
  }
  for (const candidate of candidates) {
    if (existsSync(candidate)) return candidate;
  }
  throw new Error(
    "cargo not found on PATH, in $CARGO, or under ~/.cargo/~/.rustup - install Rust or set the CARGO env var",
  );
}

// Target triple: an explicit `--target <triple>` wins (and is passed on to
// cargo); otherwise Tauri's TAURI_ENV_TARGET_TRIPLE; otherwise the rustc host.
function resolveTriple(cargo, childEnv) {
  const argv = process.argv.slice(2);
  const flag = argv.indexOf("--target");
  if (flag !== -1 && argv[flag + 1]) {
    return { triple: argv[flag + 1], explicit: true };
  }
  if (process.env.TAURI_ENV_TARGET_TRIPLE) {
    return { triple: process.env.TAURI_ENV_TARGET_TRIPLE, explicit: true };
  }
  const rustc =
    cargo === "cargo" ? "rustc" : join(dirname(cargo), `rustc${exe}`);
  const out = execFileSync(rustc, ["-vV"], { encoding: "utf8", env: childEnv });
  const match = out.match(/^host:\s*(.+)$/m);
  if (!match) {
    throw new Error("could not parse the host triple from `rustc -vV`");
  }
  return { triple: match[1].trim(), explicit: false };
}

const cargo = locateCargo();

// A cargo located off PATH (a bare toolchain binary) still shells out to its
// sibling rustc, so put its directory on PATH for every child we spawn.
const childEnv =
  cargo === "cargo"
    ? process.env
    : { ...process.env, PATH: `${dirname(cargo)}:${process.env.PATH ?? ""}` };

const { triple, explicit } = resolveTriple(cargo, childEnv);

const buildArgs = ["build", "--release", "-p", "epub-tailor"];
if (explicit) buildArgs.push("--target", triple);

console.log(`[prepare-sidecar] building epub-tailor for ${triple}`);
execFileSync(cargo, buildArgs, { cwd: repoRoot, stdio: "inherit", env: childEnv });

const releaseDir = explicit
  ? join(repoRoot, "target", triple, "release")
  : join(repoRoot, "target", "release");
const src = join(releaseDir, `epub-tailor${exe}`);
if (!existsSync(src)) {
  throw new Error(`built binary not found at ${src}`);
}

const destDir = join(appDir, "src-tauri", "binaries");
mkdirSync(destDir, { recursive: true });
const dest = join(destDir, `epub-tailor-${triple}${exe}`);
copyFileSync(src, dest);
chmodSync(dest, 0o755);

console.log(`[prepare-sidecar] copied ${src} -> ${dest}`);
