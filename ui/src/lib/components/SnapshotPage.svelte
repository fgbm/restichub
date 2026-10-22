<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import { ApiError, service } from '#lib/api/index.ts';
	import type { Agent, Repository, SnapshotItem } from '#lib/api/index.ts';
	import RestoreDialog from '#lib/components/RestoreDialog.svelte';
	import SnapshotTags from '#lib/components/SnapshotTags.svelte';
	import SnapshotTree from '#lib/components/SnapshotTree.svelte';
	import { Page } from '#lib/components/layout/index.ts';
	import { Alert, Button, EmptyState, Icon } from '#lib/components/ui/index.ts';
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

	const { repositoryId, snapshotId }: { repositoryId: string; snapshotId: string } = $props();

	let repository = $state<Repository | null>(null);
	let agents = $state<Agent[]>([]);
	let profiles = $state(new Map<string, string>());
	let snapshot = $state<SnapshotItem | null>(null);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let selection = $state(new Set<string>());
	let dialogOpen = $state(false);
	let summaryOpen = $state(false);
	let copied = $state(false);

	const canRestore = $derived(['admin', 'operator'].includes(getSession()?.role ?? ''));
	const paths = $derived(includePaths(selection));

	function describe(problem: unknown): string {
		if (problem instanceof ApiError) return tErrorCode(problem.code) ?? problem.title;
		if (problem instanceof BrowseTimeout) return problem.message;
		return t('table.error');
	}

	async function copyId(): Promise<void> {
		try {
			await navigator.clipboard.writeText(snapshotId);
			copied = true;
			setTimeout(() => (copied = false), 1500);
		} catch {
			copied = false;
		}
	}

	const title = $derived(t('restore.snapshotTitle', { id: snapshotId.slice(0, 8) }));
	// The list level keeps the filters and the target of the list, also in a new tab.
	const listQuery = $derived(
		filtersToQuery(filtersFromQuery(page.url.searchParams), {
			repository: repositoryId,
			snapshot: snapshotId,
			...targetExtra(page.url.searchParams)
		}).toString()
	);

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

<Page
	{title}
	crumbs={[{ label: repository?.name ?? '' }, { label: title }]}
	{listQuery}
	loading={canRestore && loading}
	error={canRestore ? error : null}
>
	{#if !canRestore}
		<Alert tone="attention" icon="lock" title={t('restore.forbidden')} />
	{:else if !snapshot || !repository}
		<EmptyState
			icon="restore"
			title={t('restore.snapshotMissingTitle')}
			description={t('restore.snapshotMissingDescription')}
			bordered
		/>
	{:else}
		<div class="layout">
			<aside class="side">
				<button
					type="button"
					class="side__toggle"
					aria-expanded={summaryOpen}
					aria-controls="snapshot-meta"
					onclick={() => (summaryOpen = !summaryOpen)}
				>
					<span class="side__line">
						<span class="mono">{snapshot.short_id}</span>
						<span>{snapshot.hostname}</span>
						<span>{snapshot.time ? formatLocalTime(snapshot.time) : '—'}</span>
					</span>
					<Icon name={summaryOpen ? 'chevron-up' : 'chevron-down'} size="xs" />
				</button>
				<dl id="snapshot-meta" class="meta" class:meta--open={summaryOpen}>
					<div>
						<dt>{t('restore.meta.id')}</dt>
						<dd class="id">
							<span class="mono id__text" title={snapshot.id}>{snapshot.id}</span>
							<Button
								size="sm"
								icon={copied ? 'check' : 'copy'}
								aria-label={t('restore.meta.copyId')}
								onclick={copyId}
							/>
						</dd>
					</div>
					<div>
						<dt>{t('restore.meta.time')}</dt>
						<dd>{snapshot.time ? formatLocalTime(snapshot.time) : '—'}</dd>
					</div>
					<div>
						<dt>{t('restore.meta.host')}</dt>
						<dd>{snapshot.hostname}</dd>
					</div>
					<div>
						<dt>{t('restore.meta.repository')}</dt>
						<dd>{repository.name}</dd>
					</div>
					<div>
						<dt>{t('restore.meta.paths')}</dt>
						<dd class="mono">{snapshot.paths.join(', ')}</dd>
					</div>
					{#if snapshot.total_bytes != null}
						<div>
							<dt>{t('restore.meta.content')}</dt>
							<dd>
								{t('restore.meta.contentValue', {
									files: formatCount(snapshot.total_files ?? 0),
									size: formatBytes(snapshot.total_bytes)
								})}
							</dd>
						</div>
					{/if}
					{#if snapshot.tags.length > 0}
						<div>
							<dt>{t('restore.meta.tags')}</dt>
							<dd><SnapshotTags tags={snapshot.tags} {profiles} /></dd>
						</div>
					{/if}
				</dl>
			</aside>

			<section class="files">
				<h2 class="files__head">{t('restore.files')}</h2>
				<div class="files__body">
					<SnapshotTree repositoryId={repository.id} {snapshot} bind:selected={selection} />
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
			</section>
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
</Page>

<style>
	/* The page fills the window below the top bar, the tabs and the title: only the tree scrolls. */
	.layout {
		display: grid;
		grid-template-columns: minmax(0, 1fr) 20rem;
		grid-template-areas: 'files side';
		gap: var(--rh-space-4);
		align-items: start;
		height: calc(100dvh - var(--rh-layout-topbar-height) - 12rem);
		min-height: 24rem;
	}

	.side {
		grid-area: side;
		min-width: 0;
		max-height: 100%;
		overflow-y: auto;
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-surface, var(--rh-color-canvas));
	}

	.side__toggle {
		display: none;
	}

	.meta {
		margin: 0;
		padding: var(--rh-space-1) var(--rh-space-4);
		font-size: var(--rh-font-size-sm);
	}

	.meta > div {
		padding: var(--rh-space-2) 0;
		border-top: var(--rh-border-width) solid var(--rh-color-border-muted);
	}

	.meta > div:first-child {
		border-top: 0;
	}

	.meta dt {
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-xs, var(--rh-font-size-sm));
	}

	.meta dd {
		margin: 0;
		overflow-wrap: anywhere;
	}

	.mono {
		font-family: var(--rh-font-mono, monospace);
	}

	.id {
		display: flex;
		align-items: center;
		gap: var(--rh-space-2);
	}

	.id__text {
		flex: 1;
		min-width: 0;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.files {
		display: flex;
		grid-area: files;
		flex-direction: column;
		min-width: 0;
		height: 100%;
		overflow: hidden;
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-canvas);
	}

	.files__head {
		margin: 0;
		padding: var(--rh-space-2) var(--rh-space-4);
		border-bottom: var(--rh-border-width) solid var(--rh-color-border);
		background-color: var(--rh-color-canvas-subtle);
		font-size: var(--rh-font-size-sm);
		font-weight: var(--rh-font-weight-semibold, 600);
	}

	.files__body {
		flex: 1;
		min-height: 0;
		padding: var(--rh-space-2);
	}

	.selection {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: var(--rh-space-4);
		padding: var(--rh-space-3) var(--rh-space-4);
		border-top: var(--rh-border-width) solid var(--rh-color-border);
		background-color: var(--rh-color-canvas-subtle);
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

	@media (max-width: 1023px) {
		.layout {
			grid-template-columns: minmax(0, 1fr);
			grid-template-areas:
				'side'
				'files';
			grid-template-rows: auto minmax(0, 1fr);
			align-items: stretch;
		}

		.side {
			max-height: none;
			overflow: visible;
		}

		.side__toggle {
			display: flex;
			align-items: center;
			justify-content: space-between;
			gap: var(--rh-space-3);
			width: 100%;
			padding: var(--rh-space-2) var(--rh-space-4);
			border: 0;
			background: none;
			color: inherit;
			font: inherit;
			font-size: var(--rh-font-size-sm);
			text-align: start;
			cursor: pointer;
		}

		.side__line {
			display: flex;
			flex-wrap: wrap;
			gap: 0 var(--rh-space-3);
			min-width: 0;
		}

		.meta {
			display: none;
			border-top: var(--rh-border-width) solid var(--rh-color-border-muted);
		}

		.meta--open {
			display: block;
		}
	}

	@media (max-width: 40rem) {
		.layout {
			height: calc(100dvh - var(--rh-layout-topbar-height) - 9rem);
			min-height: 30rem;
		}

		.selection {
			flex-direction: column;
			align-items: stretch;
		}

		.selection__path {
			max-width: 12rem;
		}
	}
</style>
