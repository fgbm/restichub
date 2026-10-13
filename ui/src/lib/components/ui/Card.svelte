<script lang="ts">
	import type { Snippet } from 'svelte';

	let { padded = true, children }: { padded?: boolean; children: Snippet } = $props();
</script>

<div class="card" class:card--padded={padded}>{@render children()}</div>

<style>
	.card {
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-canvas);
	}

	.card--padded {
		padding: var(--rh-space-4);
	}
</style>
