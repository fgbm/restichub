<script lang="ts">
	import type { Snippet } from 'svelte';

	let {
		title,
		description,
		count,
		bordered = false,
		children
	}: {
		title: string;
		description?: string;
		/** A counter next to the title, for example the number of rows. */
		count?: number;
		bordered?: boolean;
		children?: Snippet;
	} = $props();
</script>

<header class="page-header" class:page-header--bordered={bordered}>
	<div class="page-header__text">
		<div class="page-header__title-row">
			<h1 class="page-header__title">{title}</h1>
			{#if count !== undefined}<span class="page-header__count">{count}</span>{/if}
		</div>
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

	.page-header__title-row {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: var(--rh-space-3);
	}

	.page-header__title {
		margin: 0;
		font-size: var(--rh-font-size-xl);
		font-weight: var(--rh-font-weight-semibold);
		line-height: var(--rh-line-height-tight);
	}

	.page-header__count {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		min-width: 18px;
		height: 18px;
		padding: 0 6px;
		border-radius: var(--rh-radius-pill);
		background-color: var(--rh-color-neutral-bg);
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-xs);
		font-weight: var(--rh-font-weight-semibold);
		font-variant-numeric: tabular-nums;
		line-height: 1;
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
