<script lang="ts">
  import { app } from "$lib/states.svelte";
  import AddressBar from "./AddressBar.svelte";

  const activeTab = $derived(app.tabs.find((t) => t.label === app.activeId));
  const loading = $derived(activeTab?.state === "loading");
  const settingsShown = $derived(app.settingsOpen);
</script>

<div class="user-select-none shrink-0 border-b border-line bg-surface">
  <!-- Row 1: tabs -->
  <div class="flex h-10 items-center gap-1 overflow-x-auto px-2 [scrollbar-width:none]">
    {#each app.tabs as tab (tab.label)}
      <div
        role="button"
        tabindex="0"
        class="group flex h-7 max-w-52 min-w-0 items-center gap-1.5 rounded-md px-2 text-xs transition-colors
               {tab.label === app.activeId ? 'bg-surface-3 text-ink' : 'text-muted hover:bg-surface-2 hover:text-ink'}"
        onclick={() => app.activate(tab.label)}
        onkeydown={(e) => {
          if (e.key === "Enter" || e.key === " ") app.activate(tab.label);
        }}
      >
        <span
          class="h-2 w-2 shrink-0 rounded-full {tab.isHome
            ? 'bg-alt'
            : tab.state === 'blocked'
              ? 'bg-danger'
              : tab.state === 'loading'
                ? 'animate-pulse bg-warn'
                : 'bg-ok'}"
        ></span>
        <span class="truncate">{tab.title}</span>
        <span
          class="ml-auto hidden shrink-0 rounded p-0.5 font-mono leading-none text-faint hover:bg-surface-3 group-hover:block"
          title="Close tab"
          role="button"
          tabindex="-1"
          onclick={(e) => {
            e.stopPropagation();
            app.closeTab(tab.label);
          }}
          onkeydown={(e) => {
            if (e.key === "Enter" || e.key === " ") {
              e.stopPropagation();
              app.closeTab(tab.label);
            }
          }}
        >×</span>
      </div>
    {/each}
    <button
      class="flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-lg leading-none text-muted transition-colors hover:bg-surface-2 hover:text-ink"
      title="New tab"
      onclick={() => app.newTab()}
    >+</button>
    <div class="flex-1"></div>
    <button
      class="flex h-7 w-7 items-center justify-center rounded-md text-muted transition-colors hover:bg-surface-2 hover:text-ink {app.extractionOpen ? 'bg-surface-3 text-ink' : ''}"
      title="Extracted data"
      onclick={() => app.toggleExtraction()}
    >
      <svg viewBox="0 0 24 24" class="h-4 w-4" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M3 3v18h18" />
        <path d="M7 15l3-3 3 3 4-5" />
      </svg>
    </button>
    <button
      class="flex h-7 w-7 items-center justify-center rounded-md text-sm text-muted transition-colors hover:bg-surface-2 hover:text-ink {settingsShown ? 'bg-surface-3 text-ink' : ''}"
      title="Settings"
      onclick={() => (settingsShown ? app.closeSettings() : app.openSettings())}
    >⚙︎</button>
  </div>

  <!-- Row 2: navigation + address bar -->
  <div class="flex h-14 items-center gap-1.5 px-2">
    <button
      class="flex h-8 w-8 items-center justify-center rounded-md text-muted transition-colors hover:bg-surface-2 hover:text-ink"
      title="Back"
      onclick={() => app.back()}
    >
      <svg viewBox="0 0 24 24" class="h-4.5 w-4.5" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M19 12H5" /><path d="m12 19-7-7 7-7" />
      </svg>
    </button>
    <button
      class="flex h-8 w-8 items-center justify-center rounded-md text-muted transition-colors hover:bg-surface-2 hover:text-ink"
      title="Forward"
      onclick={() => app.forward()}
    >
      <svg viewBox="0 0 24 24" class="h-4.5 w-4.5" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M5 12h14" /><path d="m12 5 7 7-7 7" />
      </svg>
    </button>
    {#if loading}
      <button class="flex h-8 w-8 items-center justify-center rounded-md text-muted hover:bg-surface-2 hover:text-ink" title="Stop" onclick={() => app.stop()}>
        <svg viewBox="0 0 24 24" class="h-4.5 w-4.5" fill="currentColor"><rect x="6" y="6" width="12" height="12" rx="2" /></svg>
      </button>
    {:else}
      <button class="flex h-8 w-8 items-center justify-center rounded-md text-muted hover:bg-surface-2 hover:text-ink" title="Reload" onclick={() => app.reload()}>
        <svg viewBox="0 0 24 24" class="h-4.5 w-4.5" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M21 12a9 9 0 1 1-2.64-6.36" /><path d="M21 3v6h-6" />
        </svg>
      </button>
    {/if}

    <div class="min-w-0 flex-1">
      <AddressBar />
    </div>
  </div>
</div>