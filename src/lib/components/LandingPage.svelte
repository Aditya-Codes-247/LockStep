<script lang="ts">
  import { app } from "$lib/states.svelte";
  import AddressBar from "./AddressBar.svelte";

  const bookmarks = $derived(app.config?.bookmarks ?? []);
  const hero = $derived(bookmarks.slice(0, 2));
  const others = $derived(bookmarks.slice(2));

  function open(url: string) {
    app.openSite(url);
  }
</script>

<div class="flex h-full flex-col overflow-y-auto bg-bg">
  <div class="mx-auto flex w-full max-w-3xl flex-1 flex-col justify-center px-8 py-10">
    <!-- Wordmark -->
    <div class="mb-10 text-center">
      <div class="mx-auto mb-3 flex h-16 w-16 items-center justify-center overflow-hidden rounded-2xl shadow-lg">
        <img src="/lockstep-icon.png" alt="Lockstep Browser" class="h-full w-full object-contain" draggable="false" />
      </div>
      <h1 class="text-3xl font-bold tracking-tight text-ink">Lockstep Browser</h1>
      <p class="mt-1 text-sm text-muted">Only websites on the approved list can be opened.</p>
    </div>

    <!-- Prominent address bar -->
    <div class="mb-10">
      <AddressBar large />
    </div>

    <!-- Hero quick-launch -->
    {#if hero.length >= 2}
      <div class="grid grid-cols-1 gap-4 sm:grid-cols-2">
        {#each hero as bm (bm.name)}
          <button
            class="group flex h-28 items-center gap-4 rounded-2xl border border-line bg-surface p-5 text-left shadow-sm transition-transform hover:-translate-y-0.5 hover:border-accent hover:shadow-md"
            onclick={() => open(bm.url)}
          >
            <span
              class="flex h-14 w-14 shrink-0 items-center justify-center rounded-xl text-2xl font-bold text-white shadow"
              style="background:{bm.color ?? 'var(--accent)'}"
            >{bm.name.charAt(0).toUpperCase()}</span>
            <span class="min-w-0">
              <span class="block truncate text-lg font-semibold text-ink">{bm.name}</span>
              <span class="block truncate text-xs text-muted">{bm.url.replace(/^https?:\/\//, "")}</span>
            </span>
            <svg viewBox="0 0 24 24" class="ml-auto h-5 w-5 shrink-0 text-faint transition-colors group-hover:text-accent" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M5 12h14" /><path d="m12 5 7 7-7 7" />
            </svg>
          </button>
        {/each}
      </div>
    {:else}
      <p class="text-center text-xs text-faint">Add quick-launch shortcuts in Settings.</p>
    {/if}

    <!-- Remaining approved sites -->
    {#if others.length > 0}
      <div class="mt-8">
        <h2 class="mb-2 text-xs font-semibold uppercase tracking-wider text-faint">More approved sites</h2>
        <div class="flex flex-wrap gap-2">
          {#each others as bm (bm.name)}
            <button
              class="flex items-center gap-2 rounded-full border border-line bg-surface px-3 py-1.5 text-xs text-ink transition-colors hover:border-accent hover:text-accent"
              onclick={() => open(bm.url)}
            >
              <span class="h-1.5 w-1.5 rounded-full" style="background:{bm.color ?? 'var(--accent)'}"></span>
              {bm.name}
            </button>
          {/each}
        </div>
      </div>
    {/if}
  </div>
</div>