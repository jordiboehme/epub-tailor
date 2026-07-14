<script lang="ts">
  import { Command } from "@tauri-apps/plugin-shell";

  // Temporary debug scaffold: proves the bundled epub-tailor CLI runs as a
  // Tauri sidecar and that we can parse its `--report json` output. The real
  // workbench replaces all of this in the next task.
  let status = $state("");
  let busy = $state(false);

  async function listProfiles() {
    busy = true;
    status = "Running the sidecar...";
    try {
      const output = await Command.sidecar("binaries/epub-tailor", [
        "profiles",
        "--report",
        "json",
      ]).execute();
      if (output.code !== 0) {
        status = `Sidecar exited with code ${output.code}`;
        return;
      }
      const report = JSON.parse(output.stdout) as { profiles: unknown[] };
      status = `${report.profiles.length} profiles`;
    } catch (err) {
      status = `Error: ${err}`;
    } finally {
      busy = false;
    }
  }
</script>

<main
  class="flex min-h-screen flex-col items-center justify-center gap-6 bg-neutral-950 text-neutral-100"
>
  <h1 class="text-3xl font-semibold tracking-tight">EPUB Tailor</h1>
  <button
    class="rounded-lg bg-indigo-600 px-5 py-2.5 font-medium text-white transition hover:bg-indigo-500 disabled:cursor-not-allowed disabled:opacity-60"
    onclick={listProfiles}
    disabled={busy}
  >
    List profiles
  </button>
  {#if status}
    <p class="text-lg text-neutral-300">{status}</p>
  {/if}
</main>
