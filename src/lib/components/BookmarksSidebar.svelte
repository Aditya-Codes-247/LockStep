<script lang="ts">
  import { app } from "$lib/states.svelte";

  const bookmarks = $derived(app.config?.bookmarks ?? []);
  const whitelist = $derived(app.config?.whitelist ?? []);
</script>

<aside class="flex w-[216px] shrink-0 flex-col border-r border-line bg-surface">
  <div class="flex h-10 items-center gap-2 border-b border-line px-3">
    <img src="/lockstep-icon.png" alt="Lockstep" class="h-5 w-5 shrink-0 object-contain" draggable="false" />
    <span class="text-sm font-semibold tracking-tight">Bookmarks</span>
    <span class="ml-auto text-[10px] uppercase tracking-wide text-faint">{bookmarks.length}</span>
  </div>

  <nav class="flex-1 overflow-y-auto p-2">
    {#if bookmarks.length === 0}
      <p class="px-2 py-3 text-xs leading-relaxed text-faint">
        No bookmarks yet. Add approved shortcuts in Settings.
      </p>
    {:else}
      <ul class="space-y-0.5">
        {#each bookmarks as bm (bm.name)}
          <li>
            <button
              class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm text-ink transition-colors hover:bg-surface-2"
              title={bm.url}
              onclick={() => app.openSite(bm.url)}
            >
              <span
                class="flex h-5 w-5 shrink-0 items-center justify-center rounded font-bold text-white"
                style="background:{bm.color ?? 'var(--accent)'}"
              >{bm.name.charAt(0).toUpperCase()}</span>
              <span class="truncate">{bm.name}</span>
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </nav>

  <div class="border-t border-line px-3 py-2 text-[10px] text-faint">
    {whitelist.length} approved {whitelist.length === 1 ? "site" : "sites"}
  </div>
</aside>