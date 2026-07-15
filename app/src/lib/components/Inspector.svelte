<script lang="ts">
  // The right-hand panel, switched by the workbench mode: Edit shows the
  // metadata editor (in-place work), Fit shows the conversion settings
  // (always a copy). The mode itself lives in settings and is toggled in the
  // Workbench header.
  import { settings } from "../stores/settings.svelte";
  import EditInspector from "./EditInspector.svelte";
  import FitInspector from "./FitInspector.svelte";
</script>

{#if settings.mode === "edit"}
  <EditInspector />
{:else}
  <FitInspector />
{/if}
