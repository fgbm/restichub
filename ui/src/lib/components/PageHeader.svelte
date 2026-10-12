<script lang="ts">
	import type { Snippet } from 'svelte';

	let {
		title,
		description,
		children
	}: { title: string; description?: string; children?: Snippet } = $props();
</script>

<header class="page-header">
	<div class="page-header__text">
		<h1 class="page-header__title">{title}</h1>
		{#if description}
			<p class="page-header__description">{description}</p>
		{/if}
	</div>
	{#if children}
		<div class="page-header__actions">{@render children()}</div>
	{/if}
</header>

<style>
	.page-header {
		display: flex;
		flex-wrap: wrap;
		align-items: flex-start;
		justify-content: space-between;
		gap: var(--rh-space-4);
		margin-bottom: var(--rh-space-5);
	}

	.page-header__title {
		margin: 0;
		font-size: var(--rh-font-size-xl);
		font-weight: var(--rh-font-weight-semibold);
		line-height: var(--rh-line-height-tight);
	}

	.page-header__description {
		margin: var(--rh-space-1) 0 0;
		color: var(--rh-color-text-muted);
	}

	.page-header__actions {
		display: flex;
		align-items: center;
		gap: var(--rh-space-2);
	}
</style>
