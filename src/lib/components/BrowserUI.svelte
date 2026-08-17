<script lang="ts">
  import { app } from "$lib/states.svelte";
  import Chrome from "./Chrome.svelte";
  import BookmarksSidebar from "./BookmarksSidebar.svelte";
  import ExtractionSidebar from "./ExtractionSidebar.svelte";
  import LandingPage from "./LandingPage.svelte";
  import BlockPage from "./BlockPage.svelte";
  import ErrorPage from "./ErrorPage.svelte";
  import SettingsPanel from "./SettingsPanel.svelte";
  import Toast from "./Toast.svelte";

  const activeTab = $derived(app.tabs.find((t) => t.label === app.activeId));
  const blockedInfo = $derived(app.blocked);
  const showBlock = $derived(activeTab?.state === "blocked" && blockedInfo !== null);
</script>

<div class="flex h-screen w-screen flex-col overflow-hidden bg-bg text-ink">
  <Chrome />

  <div class="flex min-h-0 flex-1">
    <BookmarksSidebar />

    <main class="relative min-w-0 flex-1 overflow-hidden">
      {#if app.settingsOpen}
        <SettingsPanel />
      {:else if activeTab?.state === "home"}
        <LandingPage />
      {:else if showBlock && blockedInfo}
        <BlockPage blocked={blockedInfo} />
      {:else if activeTab?.state === "error"}
        <ErrorPage tab={activeTab} />
      {:else}
        <!-- loaded / loading site tab: the native child webview covers this area -->
        <div class="absolute inset-0 bg-bg"></div>
      {/if}
    </main>

    {#if app.extractionOpen}
      <ExtractionSidebar />
    {/if}
  </div>

  {#if app.toast}
    <Toast toast={app.toast} />
  {/if}
</div>