<script lang="ts">
  import { app } from "$lib/states.svelte";

  let { large = false }: { large?: boolean } = $props();

  const activeTab = $derived(app.tabs.find((t) => t.label === app.activeId));
  let focused = $state(false);
  let text = $state("");

  const suggestions = $derived.by(() => {
    if (!focused) return [];
    const q = text.trim().toLowerCase();
    if (!q) return [];
    return (app.config?.whitelist ?? [])
      .filter((d) => d.toLowerCase().includes(q))
      .slice(0, 6);
  });

  // Keep the input in sync with the active tab when not being edited.
  $effect(() => {
    const t = activeTab;
    if (t && !focused) {
      text = !t.isHome && t.url ? pretty(t.url) : "";
    }
  });

  function pretty(url: string): string {
    return url.replace(/^https?:\/\//, "");
  }

  function onFocus() {
    focused = true;
    app.focusAddress(true);
  }

  function onBlur() {
    // Let a suggestion click win the race.
    window.setTimeout(() => {
      focused = false;
      app.focusAddress(false);
      const t = activeTab;
      text = t && !t.isHome && t.url ? pretty(t.url) : "";
    }, 120);
  }

  async function submit() {
    if (!text.trim()) return;
    const ok = await app.navigateActive(text);
    window.setTimeout(() => {
      focused = false;
      app.focusAddress(false);
    }, 60);
    if (ok) text = "";
  }
</script>

<div class="relative {large ? 'w-full' : 'w-full'}">
  <form
    class="flex items-center gap-2 rounded-lg border bg-surface-2 px-3 focus-within:border-accent
           {large ? 'h-12 border-line-strong shadow-sm' : 'h-9 border-line'}"
    onsubmit={(e) => {
      e.preventDefault();
      submit();
    }}
  >
    <svg viewBox="0 0 24 24" class="h-4 w-4 shrink-0 text-faint" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
      <circle cx="11" cy="11" r="7" /><path d="m21 21-4.35-4.35" />
    </svg>
    <input
      type="text"
      bind:value={text}
      class="min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-faint {large ? 'text-base' : ''}"
      placeholder={large ? "Enter an approved web address…" : "Search or enter a web address…"}
      spellcheck="false"
      autocomplete="off"
      onfocus={onFocus}
      onblur={onBlur}
    />
    {#if suggestions.length > 0 && activeTab?.state !== "blocked"}
      <span class="shrink-0 rounded-full bg-surface-3 px-2 py-0.5 text-[10px] font-medium text-muted" title="Approved">
        ✓ approved
      </span>
    {/if}
  </form>

  {#if focused && suggestions.length > 0}
    <div class="absolute left-0 right-0 top-full z-50 mt-1 overflow-hidden rounded-lg border border-line bg-surface shadow-lg">
      {#each suggestions as domain (domain)}
        <button
          class="flex w-full items-center gap-2 px-3 py-2 text-left text-sm text-ink hover:bg-surface-2"
          onmousedown={(e) => {
            e.preventDefault();
            text = domain;
            submit();
          }}
        >
          <span class="h-1.5 w-1.5 rounded-full bg-ok"></span>
          <span class="truncate">{domain}</span>
          <span class="ml-auto text-[10px] text-faint">https://</span>
        </button>
      {/each}
    </div>
  {/if}
</div>