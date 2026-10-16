<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { ApiError, service } from '#lib/api/index.ts';
	import type { Agent, Repository, SnapshotItem } from '#lib/api/index.ts';
	import RestoreDialog from '#lib/components/RestoreDialog.svelte';
	import SnapshotTags from '#lib/components/SnapshotTags.svelte';
	import SnapshotTree from '#lib/components/SnapshotTree.svelte';
	import {
		Alert,
		Button,
		EmptyState,
		Icon,
		PageHeader,
		Panel,
		Skeleton
	} from '#lib/components/ui/index.ts';
	import { formatBytes, formatCount, formatLocalTime } from '#lib/format.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import {
		TARGET_PARAM,
		filtersFromQuery,
		filtersToQuery,
		includePaths,
		targetExtra
	} from '#lib/restore.ts';
	import { BrowseTimeout, settle } from '#lib/restore-poll.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';

	/** How many selected paths the footer names before it folds the rest. */
	const NAMED = 3;

	const repositoryId = page.params.repository ?? '';
	const snapshotId = page.params.snapshot ?? '';
	const filters = filtersFromQuery(page.url.searchParams);

	let repository = $state<Repository | null>(null);
	let agents = $state<Agent[]>([]);
	let profiles = $state(new Map<string, string>());
	let snapshot = $state<SnapshotItem | null>(null);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let selection = $state(new Set<string>());
	let dialogOpen = $state(false);

	const canRestore = $derived(['admin', 'operator'].includes(getSession()?.role ?? ''));
	const paths = $derived(includePaths(selection));
	const backHref = $derived(
		`${resolve('/(app)/restore')}?${filtersToQuery(filters, { repository: repositoryId, snapshot: snapshotId, ...targetExtra(page.url.searchParams) })}`
	);

	function describe(problem: unknown): string {
		if (problem instanceof ApiError) return tErrorCode(problem.code) ?? problem.title;
		if (problem instanceof BrowseTimeout) return problem.message;
		return t('table.error');
	}

	function back(event: MouseEvent): void {
		if (event.metaKey || event.ctrlKey || event.shiftKey || event.button !== 0) return;
		event.preventDefault();
		// eslint-disable-next-line svelte/no-navigation-without-resolve -- the address starts with a resolved path
		void goto(backHref);
	}

	onMount(async () => {
		try {
			const [repositories, allAgents, allProfiles, first] = await Promise.all([
				service.listRepositories(),
				service.listAgents(),
				service.listProfiles().catch(() => []),
				service.getSnapshots(repositoryId)
			]);
			repository = repositories.find((item) => item.id === repositoryId) ?? null;
			agents = allAgents;
			profiles = new Map(allProfiles.map((item) => [item.id, item.name]));
			const list = await settle(first, (item) =>
				service.getSnapshots(repositoryId, { job: item.job_id ?? undefined })
			);
			snapshot = list?.snapshots.find((item) => item.id === snapshotId) ?? null;
		} catch (problem) {
			error = describe(problem);
		} finally {
			loading = false;
		}
	});
</script>

<svelte:head>
	<title>{t('restore.title')} · {t('app.name')}</title>
</svelte:head>

<!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- the address starts with a resolved path -->
<a class="back" href={backHref} onclick={back}>
	<Icon name="arrow-left" size="xs" />
	{t('restore.back')}
</a>

{#if !canRestore}
	<Alert tone="attention" icon="lock" title={t('restore.forbidden')} />
{:else if loading}
	<Skeleton variant="row" />
{:else if error}
	<Alert tone="danger" title={error} />
{:else if !snapshot || !repository}
	<EmptyState
		icon="restore"
		title={t('restore.snapshotMissingTitle')}
		description={t('restore.snapshotMissingDescription')}
		bordered
	/>
{:else}
	<PageHeader title={t('restore.snapshotTitle', { id: snapshot.short_id })} />

	<Panel>
		<dl class="meta">
			<dt>{t('restore.meta.id')}</dt>
			<dd class="mono">{snapshot.id}</dd>
			<dt>{t('restore.meta.time')}</dt>
			<dd>{snapshot.time ? formatLocalTime(snapshot.time) : '—'}</dd>
			<dt>{t('restore.meta.host')}</dt>
			<dd>{snapshot.hostname}</dd>
			<dt>{t('restore.meta.repository')}</dt>
			<dd>{repository.name}</dd>
			<dt>{t('restore.meta.paths')}</dt>
			<dd class="mono">{snapshot.paths.join(', ')}</dd>
			{#if snapshot.total_bytes != null}
				<dt>{t('restore.meta.content')}</dt>
				<dd>
					{t('restore.meta.contentValue', {
						files: formatCount(snapshot.total_files ?? 0),
						size: formatBytes(snapshot.total_bytes)
					})}
				</dd>
			{/if}
			{#if snapshot.tags.length > 0}
				<dt>{t('restore.meta.tags')}</dt>
				<dd><SnapshotTags tags={snapshot.tags} {profiles} /></dd>
			{/if}
		</dl>
	</Panel>

	<div class="tree">
		<Panel title={t('restore.files')}>
			<SnapshotTree repositoryId={repository.id} {snapshot} bind:selected={selection} />
		</Panel>
	</div>

	<div class="selection">
		<div class="selection__text">
			<span class="selection__count">
				{paths.length === 0
					? t('restore.selectionNone')
					: t('restore.selectionSome', { count: paths.length })}
			</span>
			{#if paths.length > 0}
				<ul class="selection__paths" aria-label={t('restore.selectedList')}>
					{#each paths.slice(0, NAMED) as item (item)}
						<li class="selection__path" title={item}>{item}</li>
					{/each}
					{#if paths.length > NAMED}
						<li class="selection__more">
							{t('restore.selectionMore', { count: paths.length - NAMED })}
						</li>
					{/if}
				</ul>
			{/if}
		</div>
		<div class="selection__actions">
			{#if paths.length > 0}
				<Button variant="ghost" onclick={() => (selection = new Set())}>
					{t('restore.clearSelection')}
				</Button>
			{/if}
			<Button variant="primary" icon="restore" onclick={() => (dialogOpen = true)}>
				{t('restore.restore')}
			</Button>
		</div>
	</div>

	<RestoreDialog
		bind:open={dialogOpen}
		{repository}
		{snapshot}
		{paths}
		{agents}
		preferredAgentId={page.url.searchParams.get(TARGET_PARAM) ?? ''}
	/>
{/if}

<style>
	.back {
		display: inline-flex;
		align-items: center;
		gap: var(--rh-space-1);
		margin-bottom: var(--rh-space-3);
		color: var(--rh-color-link);
		font-size: var(--rh-font-size-sm);
		text-decoration: none;
	}

	.back:hover {
		text-decoration: underline;
	}

	.meta {
		display: grid;
		grid-template-columns: minmax(8rem, 12rem) minmax(0, 1fr);
		margin: 0;
	}

	.meta dt,
	.meta dd {
		margin: 0;
		padding: var(--rh-space-2) 0;
		border-top: var(--rh-border-width) solid var(--rh-color-border-muted);
		overflow-wrap: anywhere;
	}

	.meta dt {
		padding-inline-end: var(--rh-space-4);
		color: var(--rh-color-text-muted);
	}

	.meta dt:first-of-type,
	.meta dd:first-of-type {
		border-top: 0;
	}

	.mono {
		font-family: var(--rh-font-mono, monospace);
		font-size: var(--rh-font-size-sm);
	}

	.tree {
		margin-top: var(--rh-space-4);
	}

	.selection {
		position: sticky;
		bottom: 0;
		z-index: 2;
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: var(--rh-space-4);
		margin-top: var(--rh-space-4);
		padding: var(--rh-space-3) var(--rh-space-4);
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-surface, var(--rh-color-canvas));
		box-shadow: var(--rh-shadow-menu);
	}

	.selection__text {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-1);
		min-width: 0;
	}

	.selection__count {
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.selection__paths {
		display: flex;
		flex-wrap: wrap;
		gap: var(--rh-space-1) var(--rh-space-2);
		margin: 0;
		padding: 0;
		list-style: none;
		font-size: var(--rh-font-size-sm);
	}

	.selection__path {
		max-width: 22rem;
		overflow: hidden;
		padding: 0 var(--rh-space-2);
		border-radius: var(--rh-radius-sm);
		background-color: var(--rh-color-hover);
		font-family: var(--rh-font-mono, monospace);
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.selection__more {
		color: var(--rh-color-text-muted);
	}

	.selection__actions {
		display: inline-flex;
		flex: none;
		align-items: center;
		gap: var(--rh-space-2);
	}

	@media (max-width: 40rem) {
		.selection {
			flex-direction: column;
			align-items: stretch;
		}

		.selection__path {
			max-width: 12rem;
		}

		.meta {
			grid-template-columns: minmax(0, 1fr);
		}

		.meta dt {
			padding-bottom: 0;
			border-top: var(--rh-border-width) solid var(--rh-color-border-muted);
		}

		.meta dd {
			padding-top: var(--rh-space-1);
			border-top: 0;
		}
	}
</style>
