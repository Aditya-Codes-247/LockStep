<script lang="ts">
  import { app } from "$lib/states.svelte";
  import type { Tab } from "$lib/types";

  let { tab }: { tab: Tab } = $props();
</script>

<div class="flex h-full flex-col items-center justify-center gap-3 bg-bg p-8 text-center">
  <svg viewBox="0 0 24 24" class="h-12 w-12 text-warn" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
    <circle cx="12" cy="12" r="9" /><path d="M12 8v4M12 16h.01" />
  </svg>
  <h2 class="text-lg font-semibold text-ink">We couldn't load this page</h2>
  <p class="max-w-md break-all text-sm text-muted">{tab.url}</p>
  <button
    class="mt-2 rounded-lg bg-ink px-4 py-2 text-sm font-medium text-surface transition-opacity hover:opacity-90"
    onclick={() => app.reload()}
  >Try again</button>
</div>