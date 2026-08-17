<script lang="ts">
  import { app } from "$lib/states.svelte";
  import type { BlockedInfo } from "$lib/types";

  let { blocked }: { blocked: BlockedInfo } = $props();
</script>

<div class="flex h-full items-center justify-center bg-danger-soft p-8">
  <div class="w-full max-w-md rounded-2xl border border-danger bg-surface p-8 text-center shadow-lg">
    <div class="mx-auto mb-4 flex h-16 w-16 items-center justify-center rounded-full bg-danger/15">
      <svg viewBox="0 0 24 24" class="h-9 w-9 text-danger" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M12 3 4 7v5c0 4.7 3.4 8.9 8 10 4.6-1.1 8-5.3 8-10V7l-8-4Z" />
        <path d="M10 10l4 4M14 10l-4 4" />
      </svg>
    </div>

    <h1 class="text-xl font-bold text-ink">This site is not on the approved list</h1>
    <p class="mt-2 break-all font-mono text-sm text-muted">{blocked.url}</p>

    <div class="mt-4 rounded-lg bg-danger-soft px-4 py-3 text-left text-sm text-danger">
      {blocked.reason}
    </div>

    <div class="mt-6 flex justify-center gap-2">
      <button
        class="rounded-lg bg-ink px-4 py-2 text-sm font-medium text-surface transition-opacity hover:opacity-90"
        onclick={() => app.dismissBlock()}
      >Got it</button>
      <button
        class="rounded-lg border border-line px-4 py-2 text-sm font-medium text-ink transition-colors hover:bg-surface-2"
        onclick={() => {
          app.dismissBlock();
          app.goHome();
        }}
      >Go to approved home</button>
    </div>
  </div>
</div>