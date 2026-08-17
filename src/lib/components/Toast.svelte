<script lang="ts">
  import type { Toast } from "$lib/states.svelte";

  let { toast }: { toast: Toast } = $props();

  const badge = $derived(
    toast.kind === "ok"
      ? "bg-ok text-white"
      : toast.kind === "warn"
        ? "bg-warn text-white"
        : "bg-danger text-white",
  );
</script>

<div class="absolute bottom-4 right-4 z-[100] flex items-center gap-2 rounded-lg border border-line bg-surface px-4 py-3 text-sm text-ink shadow-lg">
  <span class={`flex h-5 w-5 items-center justify-center rounded-full text-xs font-bold ${badge}`}>!</span>
  <span class="max-w-xs">{toast.message}</span>
</div>