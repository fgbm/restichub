<script lang="ts">
	import type { Snippet } from 'svelte';

	let { children }: { children: Snippet } = $props();
</script>

<main class="auth">
	<div class="auth__card">{@render children()}</div>
</main>

<style>
	.auth {
		display: flex;
		align-items: center;
		justify-content: center;
		min-height: 100vh;
		padding: var(--rh-space-6) var(--rh-space-4);
	}

	.auth__card {
		width: 100%;
		max-width: 24rem;
		padding: var(--rh-space-6);
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-surface);
	}
</style>
