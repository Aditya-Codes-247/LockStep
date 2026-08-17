<script lang="ts">
  import { app } from "$lib/states.svelte";

  const config = $derived(app.config);
  const whitelist = $derived(config?.whitelist ?? []);
  const bookmarks = $derived(config?.bookmarks ?? []);
  const settings = $derived(config?.settings);

  // local form state
  let newDomain = $state("");
  let bmName = $state("");
  let bmUrl = $state("");
  let bmColor = $state("#387ed1");
  let homeUrlInput = $state("");

  $effect(() => {
    if (homeUrlInput === "" && settings) homeUrlInput = settings.homeUrl;
  });

  function toggleTheme() {
    const next = app.config?.settings.theme === "dark" ? "light" : "dark";
    app.setTheme(next);
  }

  async function addShortcut() {
    const name = bmName.trim();
    const url = bmUrl.trim();
    if (!name || !url) {
      app.flash("Enter a name and a web address.", "error");
      return;
    }
    const ok = await app.addBookmark(name, url, bmColor);
    if (ok) {
      bmName = "";
      bmUrl = "";
    }
  }
</script>

<div class="h-full overflow-y-auto bg-bg">
  <div class="mx-auto max-w-2xl px-6 py-8">
    <div class="mb-6 flex items-center gap-2">
      <svg viewBox="0 0 24 24" class="h-5 w-5 text-muted" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09a1.65 1.65 0 0 0-1-1.51 1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09a1.65 1.65 0 0 0 1.51-1 1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33h.03a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51h.03a1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82v.03a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1Z" />
      </svg>
      <h1 class="text-xl font-bold text-ink">Settings</h1>
      <button class="ml-auto rounded-lg bg-ink px-4 py-2 text-sm font-medium text-surface transition-opacity hover:opacity-90" onclick={() => app.closeSettings()}>
        Done
      </button>
    </div>

    {#if !config}
      <p class="text-sm text-muted">Loading…</p>
    {:else}
      <!-- Appearance -->
      <section class="mb-6 rounded-xl border border-line bg-surface p-5">
        <h2 class="mb-3 text-sm font-semibold text-ink">Appearance</h2>
        <div class="flex gap-2">
          <button
            class="flex-1 rounded-lg border px-4 py-2.5 text-sm font-medium transition-colors
              {settings?.theme === 'dark' ? 'border-accent bg-accent/15 text-accent' : 'border-line text-muted hover:bg-surface-2'}"
            onclick={() => toggleTheme()}
          >
            Dark {settings?.theme === "dark" ? "✓" : ""}
          </button>
          <button
            class="flex-1 rounded-lg border px-4 py-2.5 text-sm font-medium transition-colors
              {settings?.theme === 'light' ? 'border-accent bg-accent/15 text-accent' : 'border-line text-muted hover:bg-surface-2'}"
            onclick={() => app.config?.settings.theme !== "light" && app.setTheme("light")}
          >
            Light {settings?.theme === "light" ? "✓" : ""}
          </button>
        </div>
      </section>

      <!-- Homepage -->
      <section class="mb-6 rounded-xl border border-line bg-surface p-5">
        <h2 class="mb-3 text-sm font-semibold text-ink">Default homepage</h2>
        <div class="flex flex-col gap-3">
          <label class="flex items-center gap-2 text-sm text-ink">
            <input type="radio" checked={settings?.homepage === "landing"} onchange={() => app.setHomepage("landing")} class="accent-accent" />
            Landing page (quick-launch)
          </label>
          <label class="flex items-center gap-2 text-sm text-ink">
            <input type="radio" checked={settings?.homepage === "custom"} onchange={() => app.setHomepage("custom")} class="accent-accent" />
            Custom site
          </label>
          {#if settings?.homepage === "custom"}
            <div class="flex gap-2">
              <input
                type="text"
                bind:value={homeUrlInput}
                class="min-w-0 flex-1 rounded-lg border border-line bg-surface-2 px-3 py-2 text-sm text-ink outline-none focus:border-accent"
                placeholder="e.g. https://coinswitch.co"
                onchange={() => homeUrlInput && app.setHomeUrl(homeUrlInput)}
              />
              <button class="rounded-lg bg-ink px-4 py-2 text-sm font-medium text-surface transition-opacity hover:opacity-90" onclick={() => homeUrlInput && app.setHomeUrl(homeUrlInput)}>
                Save
              </button>
            </div>
          {/if}
        </div>
      </section>

      <!-- Whitelist -->
      <section class="mb-6 rounded-xl border border-line bg-surface p-5">
        <h2 class="mb-3 text-sm font-semibold text-ink">Approved websites</h2>
        <p class="mb-3 text-xs text-muted">
          Only these hostnames (and their subdomains) may be visited. Try to open anything else and you'll get the block page.
        </p>
        <div class="mb-3 flex flex-wrap gap-2">
          {#each whitelist as domain (domain)}
            <span class="flex items-center gap-1.5 rounded-full border border-line bg-surface-2 px-3 py-1 text-xs text-ink">
              <span class="h-1.5 w-1.5 rounded-full bg-ok"></span>
              {domain}
              <button class="text-faint hover:text-danger" title="Remove" onclick={() => app.removeDomain(domain)}>×</button>
            </span>
          {/each}
        </div>
        <div class="flex gap-2">
          <input
            type="text"
            bind:value={newDomain}
            class="min-w-0 flex-1 rounded-lg border border-line bg-surface-2 px-3 py-2 text-sm text-ink outline-none focus:border-accent"
            placeholder="example.com (hostname only)"
            onkeydown={(e) => {
              if (e.key === "Enter" && newDomain.trim()) {
                app.addDomain(newDomain);
                newDomain = "";
              }
            }}
          />
          <button
            class="rounded-lg bg-ink px-4 py-2 text-sm font-medium text-surface transition-opacity hover:opacity-90"
            onclick={() => {
              if (newDomain.trim()) {
                app.addDomain(newDomain);
                newDomain = "";
              }
            }}
          >Add</button>
        </div>
      </section>

      <!-- Shortcuts -->
      <section class="mb-6 rounded-xl border border-line bg-surface p-5">
        <h2 class="mb-3 text-sm font-semibold text-ink">Quick-launch shortcuts</h2>
        <ul class="mb-4 space-y-1.5">
          {#each bookmarks as bm (bm.name)}
            <li class="flex items-center gap-2 rounded-lg bg-surface-2 px-3 py-2">
              <span class="flex h-6 w-6 items-center justify-center rounded text-xs font-bold text-white" style="background:{bm.color ?? 'var(--accent)'}">
                {bm.name.charAt(0).toUpperCase()}
              </span>
              <span class="min-w-0 flex-1">
                <span class="block truncate text-sm font-medium text-ink">{bm.name}</span>
                <span class="block truncate text-[11px] text-muted">{bm.url}</span>
              </span>
              <button class="rounded p-1 text-faint transition-colors hover:bg-surface-3 hover:text-danger" title="Remove shortcut" onclick={() => app.removeBookmark(bm.name)}>
                <svg viewBox="0 0 24 24" class="h-4 w-4" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M3 6h18" /><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6" /><path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
                </svg>
              </button>
            </li>
          {/each}
        </ul>
        <div class="flex flex-col gap-2 sm:flex-row">
          <input type="text" bind:value={bmName} class="min-w-0 flex-1 rounded-lg border border-line bg-surface-2 px-3 py-2 text-sm outline-none focus:border-accent" placeholder="Shortcut name" />
          <input type="text" bind:value={bmUrl} class="min-w-0 flex-1 rounded-lg border border-line bg-surface-2 px-3 py-2 text-sm outline-none focus:border-accent" placeholder="https://example.com" />
          <input type="color" bind:value={bmColor} class="h-10 w-12 self-center rounded cursor-pointer border border-line bg-transparent" title="Shortcut color" />
          <button class="rounded-lg bg-ink px-4 py-2 text-sm font-medium text-surface transition-opacity hover:opacity-90" onclick={addShortcut}>Add</button>
        </div>
      </section>

      <p class="text-center text-[11px] text-faint">
        Config stored at <span class="font-mono">config.json</span> in the app data folder — edit it anytime.
      </p>
    {/if}
  </div>
</div>