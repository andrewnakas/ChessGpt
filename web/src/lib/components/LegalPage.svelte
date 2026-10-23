<script lang="ts">
  import type { Snippet } from 'svelte';
  let { title, updated, children }: { title: string; updated: string; children: Snippet } = $props();
</script>

<svelte:head><title>{title} · chessgpt</title></svelte:head>

<article class="card">
  <h1>{title}</h1>
  <p class="muted small">Last updated {updated}</p>
  {@render children()}
</article>

<style>
  article {
    max-width: 48rem;
    padding: 1.2rem 1.6rem;
    line-height: 1.55;
  }
  article :global(h1) {
    margin-top: 0;
    font-size: 1.5rem;
  }
  article :global(h2) {
    font-size: 1.1rem;
    margin: 1.4rem 0 0.3rem;
  }
  article :global(li) {
    margin: 0.25rem 0;
  }
  .small {
    font-size: 0.85rem;
  }
</style>
