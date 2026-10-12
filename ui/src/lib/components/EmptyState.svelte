<script lang="ts">
	import type { Snippet } from 'svelte';

	let {
		title,
		description,
		children
	}: { title: string; description?: string; children?: Snippet } = $props();
</script>

<div class="empty-state">
	<p class="empty-state__title">{title}</p>
	{#if description}
		<p class="empty-state__description">{description}</p>
	{/if}
	{#if children}
		<div class="empty-state__actions">{@render children()}</div>
	{/if}
</div>

<style>
	.empty-state {
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: var(--rh-space-2);
		padding: var(--rh-space-10) var(--rh-space-4);
		text-align: center;
	}

	.empty-state__title {
		margin: 0;
		font-size: var(--rh-font-size-md);
		font-weight: var(--rh-font-weight-semibold);
		color: var(--rh-color-text);
	}

	.empty-state__description {
		margin: 0;
		max-width: 40ch;
		color: var(--rh-color-text-muted);
	}

	.empty-state__actions {
		margin-top: var(--rh-space-2);
	}
</style>
