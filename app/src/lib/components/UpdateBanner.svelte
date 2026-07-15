<script lang="ts">
  // The auto-updater's entire user interface: one strip at the top of the
  // window, shown only when a newer release actually exists. Everything about
  // it is deliberately quiet - an update is an offer, not an interruption, and
  // a person mid-conversion should be able to ignore it forever.
  //
  // The update itself comes from `latest.json`, attached to every published
  // GitHub release by the release workflow and signed with the updater key.
  // The plugin verifies that signature against the public key baked into
  // tauri.conf.json before it installs anything.
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { check, type Update } from "@tauri-apps/plugin-updater";
  import { relaunch } from "@tauri-apps/plugin-process";
  import { slide } from "svelte/transition";
  import Button from "./ui/Button.svelte";

  type Phase = "idle" | "offered" | "downloading" | "installed" | "failed";

  let phase = $state<Phase>("idle");
  let update: Update | null = null;
  let version = $state("");
  let percent = $state<number | null>(null);

  /**
   * Whether this build is allowed to update itself in place.
   *
   * A dev build must not: it would happily replace the app you are debugging
   * with the last release. A Linux .deb must not either - apt owns those files
   * and the updater cannot write them; only the AppImage is self-contained
   * enough to swap itself out, and `is_appimage` (a Rust command) is what knows
   * the difference. The webview's user agent is the cheapest reliable way to
   * ask which OS we are on, and it is only ever consulted to answer that.
   */
  async function updatable(): Promise<boolean> {
    if (!import.meta.env.PROD) return false;
    const ua = navigator.userAgent;
    const linux = ua.includes("Linux") && !ua.includes("Mac OS X");
    if (!linux) return true;
    return await invoke<boolean>("is_appimage");
  }

  onMount(() => {
    void (async () => {
      try {
        if (!(await updatable())) return;
        const found = await check();
        if (!found) return;
        update = found;
        version = found.version;
        phase = "offered";
      } catch {
        // A failed check is a non-event: no network, a rate limit, a release
        // without a latest.json. The app works exactly as well as it did a
        // second ago, so it says nothing at all.
      }
    })();
  });

  async function install() {
    if (!update) return;
    phase = "downloading";
    percent = 0;
    let total = 0;
    let done = 0;
    try {
      await update.downloadAndInstall((event) => {
        switch (event.event) {
          case "Started":
            total = event.data.contentLength ?? 0;
            break;
          case "Progress":
            done += event.data.chunkLength;
            percent = total > 0 ? Math.min(100, Math.round((done / total) * 100)) : null;
            break;
          case "Finished":
            percent = 100;
            break;
        }
      });
      phase = "installed";
      await relaunch();
    } catch {
      // Quiet, like the check: the current version is still perfectly good.
      phase = "failed";
    }
  }
</script>

{#if phase !== "idle"}
  <div
    transition:slide={{ duration: 140 }}
    class="flex shrink-0 items-center gap-3 border-b border-teal-200 bg-teal-50 px-4 py-2 text-[12px] leading-snug text-teal-900 dark:border-teal-500/30 dark:bg-teal-500/10 dark:text-teal-100"
  >
    {#if phase === "offered"}
      <span class="min-w-0 flex-1">
        Version {version} is ready to try on. Fresh seams, same fit.
      </span>
      <Button size="sm" variant="ghost" onclick={() => (phase = "idle")}>Later</Button>
      <Button size="sm" variant="primary" onclick={install}>Install</Button>
    {:else if phase === "downloading"}
      <span class="min-w-0 flex-1">
        Taking in version {version}{percent === null ? "" : ` - ${percent}%`}. The app restarts when it fits.
      </span>
      {#if percent !== null}
        <div
          class="h-1 w-24 shrink-0 overflow-hidden rounded-full bg-teal-200 dark:bg-teal-500/25"
          role="progressbar"
          aria-label="Downloading the update"
          aria-valuenow={percent}
          aria-valuemin={0}
          aria-valuemax={100}
        >
          <div class="h-full bg-teal-600 transition-all dark:bg-teal-400 dark:shadow-glow-sm" style="width: {percent}%"></div>
        </div>
      {/if}
    {:else if phase === "installed"}
      <span class="min-w-0 flex-1">Version {version} is in. Restarting.</span>
    {:else if phase === "failed"}
      <span class="min-w-0 flex-1">
        Version {version} would not go on - the update was left alone and this one still fits. Try again later, or
        download it yourself.
      </span>
      <Button size="sm" variant="ghost" onclick={() => (phase = "idle")}>Dismiss</Button>
    {/if}
  </div>
{/if}
