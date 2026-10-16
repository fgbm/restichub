<script lang="ts">
	import type { Snippet } from 'svelte';

	let {
		title,
		description,
		count,
		status,
		context,
		children
	}: {
		title: string;
		description?: string;
		/** A counter next to the title, for example the number of rows. */
		count?: number;
		/** A status pill next to the title. */
		status?: Snippet;
		/** The page context, for example the repository that the list belongs to. */
		context?: Snippet;
		/** The primary actions of the page. */
		children?: Snippet;
	} = $props();
</script>

<header class="page-header">
	<div class="page-header__title-row">
		<h1 class="page-header__title">{title}</h1>
		{#if count !== undefined}<span class="page-header__count">{count}</span>{/if}
		{@render status?.()}
	</div>
	{#if context || children}
		<div class="page-header__side">
			{#if context}
				<div class="page-header__context">{@render context()}</div>
			{/if}
			{#if children}
				<div class="page-header__actions">{@render children()}</div>
			{/if}
		</div>
	{/if}
	{#if description}
		<p class="page-header__description">{description}</p>
	{/if}
</header>

<style>
	.page-header {
		display: grid;
		grid-template-columns: minmax(0, 1fr) auto;
		align-items: center;
		column-gap: var(--rh-space-4);
		padding-bottom: var(--rh-space-6);
	}

	.page-header__title-row {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: var(--rh-space-3);
		min-width: 0;
	}

	.page-header__title {
		margin: 0;
		font-size: var(--rh-font-size-xl);
		font-weight: var(--rh-font-weight-semibold);
		line-height: var(--rh-line-height-tight);
		overflow-wrap: anywhere;
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
		grid-column: 1;
		margin: var(--rh-space-1) 0 0;
		color: var(--rh-color-text-muted);
	}

	.page-header__side {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		justify-content: flex-end;
		gap: var(--rh-space-3);
		min-width: 0;
	}

	.page-header__context {
		display: flex;
		align-items: center;
		gap: var(--rh-space-2);
		min-width: 0;
	}

	.page-header__context :global(label) {
		color: var(--rh-color-text-muted);
		white-space: nowrap;
	}

	.page-header__actions {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		justify-content: flex-end;
		gap: var(--rh-space-2);
	}

	@media (max-width: 720px) {
		.page-header {
			grid-template-columns: minmax(0, 1fr);
		}

		.page-header__side {
			grid-row: 3;
			justify-content: flex-start;
			margin-top: var(--rh-space-3);
		}

		.page-header__context {
			flex: 1 1 100%;
		}

		.page-header__context > :global(*) {
			flex: 1 1 auto;
		}

		.page-header__actions {
			justify-content: flex-start;
		}
	}
</style>
