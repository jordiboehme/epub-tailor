<script lang="ts">
  // The right-hand panel, switched by the workbench mode: Edit shows the
  // metadata editor (in-place work), Fit shows the conversion settings
  // (always a copy). The mode itself lives in settings and is toggled in the
  // Workbench header.
  import { settings } from "../stores/settings.svelte";
  import ConditionPanel from "./ConditionPanel.svelte";
  import EditInspector from "./EditInspector.svelte";
  import FitInspector from "./FitInspector.svelte";
</script>

<!-- Above the mode panel: "does this need anything?" is not a mode, and the
     app opens in Fit, where there was otherwise no way to act on a verdict. -->
<ConditionPanel />

{#if settings.mode === "edit"}
  <EditInspector />
{:else}
  <FitInspector />
{/if}
