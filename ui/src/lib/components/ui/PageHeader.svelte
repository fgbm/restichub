<script lang="ts">
	import type { Snippet } from 'svelte';

	let {
		title,
		description,
		bordered = false,
		children
	}: {
		title: string;
		description?: string;
		bordered?: boolean;
		children?: Snippet;
	} = $props();
</script>

<header class="page-header" class:page-header--bordered={bordered}>
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
		align-items: flex-start;
		gap: var(--rh-space-4);
		padding-bottom: var(--rh-space-4);
	}

	.page-header--bordered {
		border-bottom: var(--rh-border-width) solid var(--rh-color-border);
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
		margin-left: auto;
		flex-wrap: wrap;
		justify-content: flex-end;
	}
</style>
