<script lang="ts">
	import type { Snippet } from 'svelte';
	import type { Crumb } from '#lib/stores/page-meta.svelte.ts';
	import Page from './Page.svelte';

	let {
		title,
		crumbs,
		listQuery,
		description,
		loading = false,
		error = null,
		status,
		actions,
		banners,
		side,
		footer,
		onsubmit,
		children
	}: {
		title: string;
		/** The breadcrumb levels below the section. Default: the title as the only level. */
		crumbs?: Crumb[];
		listQuery?: string;
		description?: string;
		loading?: boolean;
		error?: string | null;
		status?: Snippet;
		actions?: Snippet;
		/** Notices about the whole record, above the sections. */
		banners?: Snippet;
		/** The sticky side panel: the summary or the parameters of the record. */
		side?: Snippet;
		/** A sticky bar below the sections, such as the save bar of an editor. */
		footer?: Snippet;
		/** Makes the sections and the footer one form. */
		onsubmit?: (event: SubmitEvent) => void;
		/** The sections of the record. */
		children: Snippet;
	} = $props();
</script>

<Page
	{title}
	crumbs={crumbs ?? [{ label: title }]}
	{listQuery}
	{description}
	{loading}
	{error}
	{status}
	{actions}
>
	{#if banners}
		<div class="record__banners">{@render banners()}</div>
	{/if}
	<svelte:element
		this={onsubmit ? 'form' : 'div'}
		class="record"
		class:record--side={Boolean(side)}
		onsubmit={onsubmit
			? (event: SubmitEvent) => {
					event.preventDefault();
					onsubmit?.(event);
				}
			: undefined}
	>
		<div class="record__main">{@render children()}</div>
		{#if side}
			<aside class="record__side">{@render side()}</aside>
		{/if}
		{#if footer}
			<div class="record__footer">{@render footer()}</div>
		{/if}
	</svelte:element>
</Page>

<style>
	.record__banners {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-3);
		margin-bottom: var(--rh-space-4);
	}

	.record {
		display: grid;
		grid-template-columns: minmax(0, 1fr);
		grid-template-areas:
			'main'
			'bar';
		gap: var(--rh-space-4);
		align-items: start;
	}

	.record--side {
		grid-template-columns: minmax(0, 1fr) var(--rh-layout-side-width);
		grid-template-areas:
			'main side'
			'bar bar';
	}

	.record__main {
		grid-area: main;
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-3);
		min-width: 0;
	}

	.record__side {
		grid-area: side;
		position: sticky;
		top: calc(var(--rh-layout-topbar-height) + var(--rh-space-4));
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-3);
		min-width: 0;
		max-height: calc(100vh - var(--rh-layout-topbar-height) - var(--rh-space-8));
		overflow-y: auto;
	}

	.record__footer {
		position: sticky;
		bottom: 0;
		z-index: var(--rh-z-sticky);
		grid-area: bar;
	}

	@media (max-width: 1023px) {
		.record--side {
			grid-template-columns: minmax(0, 1fr);
			grid-template-areas:
				'side'
				'main'
				'bar';
		}

		.record__side {
			position: static;
			max-height: none;
		}
	}
</style>
