<script lang="ts">
  import "../app.css";
  import { onMount } from "svelte";
  import { app } from "$lib/states.svelte";

  onMount(() => {
    app.init();
  });
</script>

<svelte:head>
  <title>Lockstep Browser</title>
</svelte:head>

<slot />