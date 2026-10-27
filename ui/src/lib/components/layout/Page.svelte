<script lang="ts">
	import type { Snippet } from 'svelte';
	import type { Crumb } from '#lib/stores/page-meta.svelte.ts';
	import { Alert, PageHeader, Skeleton } from '../ui/index.ts';
	import { showSkeleton } from '#lib/skeleton.svelte.ts';
	import PageMeta from './PageMeta.svelte';

	let {
		title,
		documentTitle,
		crumbs = [],
		listQuery,
		description,
		count,
		loading = false,
		error = null,
		context,
		status,
		actions,
		children
	}: {
		/** The heading and the document title. */
		title: string;
		/** The document title when it says more than the heading, such as the platform mode. */
		documentTitle?: string;
		/** The breadcrumb levels below the section; the last one is this page. A list page has none. */
		crumbs?: Crumb[];
		/** The query of the section level of the breadcrumb, for an address opened in a new tab. */
		listQuery?: string;
		description?: string;
		count?: number | string;
		/** While the record loads, the page shows a placeholder instead of the header and the body. */
		loading?: boolean;
		error?: string | null;
		/** The page context next to the title, for example the repository that a list belongs to. */
		context?: Snippet;
		/** A status pill next to the title. */
		status?: Snippet;
		actions?: Snippet;
		children: Snippet;
	} = $props();

	const skeleton = showSkeleton(() => loading);
</script>

<PageMeta title={documentTitle ?? title} {crumbs} {listQuery} />

{#if loading}
	{#if skeleton()}
		<div class="page-loading" aria-busy="true">
			<Skeleton />
			<Skeleton variant="row" />
		</div>
	{/if}
{:else if error}
	<Alert tone="danger" title={error} />
{:else}
	<PageHeader {title} {description} {count} {context} {status} children={actions} />
	{@render children()}
{/if}

<style>
	.page-loading {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-3);
		max-width: 480px;
	}
</style>
