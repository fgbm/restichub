<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { ApiError, service } from '#lib/api/index.ts';
	import type { Repository, SnapshotItem, SnapshotsView } from '#lib/api/index.ts';
	import SnapshotFilterBar from '#lib/components/SnapshotFilterBar.svelte';
	import SnapshotTags from '#lib/components/SnapshotTags.svelte';
	import {
		Alert,
		Button,
		EmptyState,
		PageHeader,
		Panel,
		Select,
		Skeleton
	} from '#lib/components/ui/index.ts';
	import type { SelectOption } from '#lib/components/ui/index.ts';
	import { formatBytes, formatLocalTime, relativeTime } from '#lib/format.ts';
	import { t, tErrorCode, tOr } from '#lib/i18n/index.ts';
	import { EMPTY_FILTERS, filtersFromQuery, filtersToQuery, targetExtra } from '#lib/restore.ts';
	import type { RestoreFilters } from '#lib/restore.ts';
	import { BrowseTimeout, settle } from '#lib/restore-poll.ts';
	import { applyFilters, hasFilters } from '#lib/snapshot-filter.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';

	const PAGE_SIZE = 15;
	const ADDRESS_DELAY = 300;

	const query = page.url.searchParams;
	// The snapshot that the person had open: the detail page links back with it.
	const opened = query.get('snapshot') ?? '';

	let repositories = $state<Repository[]>([]);
	let profiles = $state(new Map<string, string>());
	let repositoryId = $state(query.get('repository') ?? '');
	let filters = $state<RestoreFilters>(filtersFromQuery(query));

	let loading = $state(true);
	let pending = $state(false);
	let error = $state<string | null>(null);
	let view = $state<SnapshotsView | null>(null);
	let visible = $state(PAGE_SIZE);
	// A newer request makes the answer of an older one obsolete.
	let generation = 0;
	let addressTimer: ReturnType<typeof setTimeout> | undefined;

	const canRestore = $derived(['admin', 'operator'].includes(getSession()?.role ?? ''));
	const repositoryOptions = $derived<SelectOption[]>(
		repositories.map((item) => ({ value: item.id, label: item.name }))
	);
	const snapshots = $derived(view?.state === 'done' ? view.snapshots : []);
	const matching = $derived(applyFilters(snapshots, filters, Date.now()));
	const shown = $derived(matching.slice(0, visible));

	function describe(problem: unknown): string {
		if (problem instanceof ApiError) return tErrorCode(problem.code) ?? problem.title;
		if (problem instanceof BrowseTimeout) return problem.message;
		return t('table.error');
	}

	function reasonOf(code: string | null | undefined): string {
		if (!code) return '';
		return tOr(`jobs.reason.${code}`, code);
	}

	// The screen filters the whole list itself, so only a list that restic made without filters fits.
	function isWhole(stored: SnapshotsView): boolean {
		return stored.hosts.length === 0 && stored.tags.length === 0 && stored.paths.length === 0;
	}

	function remember(): void {
		const address = `${resolve('/(app)/restore')}?${filtersToQuery(filters, { repository: repositoryId, ...targetExtra(query) })}`;
		// eslint-disable-next-line svelte/no-navigation-without-resolve -- the address starts with a resolved path
		void goto(address, { replace: true, reset: false });
	}

	function rememberSoon(): void {
		clearTimeout(addressTimer);
		addressTimer = setTimeout(remember, ADDRESS_DELAY);
	}

	/** Reads the stored list of the repository. A repository without a whole one gets a request. */
	async function open(): Promise<void> {
		generation += 1;
		const mine = generation;
		view = null;
		error = null;
		if (!repositoryId) return;
		try {
			const stored = await service.getSnapshots(repositoryId);
			if (mine !== generation) return;
			if (stored.state === 'empty' || !isWhole(stored)) await find();
			else {
				view = stored;
				visible = PAGE_SIZE;
			}
		} catch (problem) {
			if (mine === generation) error = describe(problem);
		}
	}

	/** Runs a list job for the whole repository. The old list stays on screen until the new one is in. */
	async function find(): Promise<void> {
		generation += 1;
		const mine = generation;
		pending = true;
		error = null;
		try {
			const started = await service.startSnapshots(repositoryId, {
				hosts: [],
				tags: [],
				paths: []
			});
			if (mine !== generation) return;
			const done = await settle(
				started,
				(item) => service.getSnapshots(repositoryId, { job: item.job_id ?? undefined }),
				{ cancelled: () => mine !== generation }
			);
			if (done && mine === generation) {
				view = done;
				visible = PAGE_SIZE;
			}
		} catch (problem) {
			if (mine === generation) error = describe(problem);
		} finally {
			if (mine === generation) pending = false;
		}
	}

	function change(patch: Partial<RestoreFilters>): void {
		filters = { ...filters, ...patch };
		visible = PAGE_SIZE;
		rememberSoon();
	}

	function reset(): void {
		filters = { ...EMPTY_FILTERS };
		visible = PAGE_SIZE;
		rememberSoon();
	}

	function changeRepository(): void {
		void open().then(remember);
	}

	// The address starts with a resolved path; only the query string follows it.
	function detailHref(snapshot: SnapshotItem): string {
		const address = resolve('/(app)/restore/[repository]/[snapshot]', {
			repository: repositoryId,
			snapshot: snapshot.id
		});
		return `${address}?${filtersToQuery(filters, targetExtra(query))}`;
	}

	onMount(() => {
		void (async () => {
			try {
				const [allRepositories, allProfiles] = await Promise.all([
					service.listRepositories(),
					service.listProfiles().catch(() => [])
				]);
				repositories = allRepositories.filter((item) => item.initialized);
				profiles = new Map(allProfiles.map((item) => [item.id, item.name]));
				if (!repositories.some((item) => item.id === repositoryId)) {
					repositoryId = repositories[0]?.id ?? '';
				}
			} catch (problem) {
				error = describe(problem);
			} finally {
				loading = false;
			}
			await open();
		})();
		return () => clearTimeout(addressTimer);
	});
</script>

<svelte:head>
	<title>{t('restore.title')} · {t('app.name')}</title>
</svelte:head>

<PageHeader title={t('restore.title')} description={t('restore.description')}>
	{#if canRestore && repositories.length > 0}
		<div class="repository">
			<label class="repository__label" for="restore-repository"
				>{t('restore.repositoryLabel')}</label
			>
			<Select
				id="restore-repository"
				options={repositoryOptions}
				bind:value={repositoryId}
				onchange={changeRepository}
			/>
		</div>
	{/if}
</PageHeader>

{#if !canRestore}
	<Alert tone="attention" icon="lock" title={t('restore.forbidden')} />
{:else if !loading && repositories.length === 0}
	<EmptyState
		icon="restore"
		title={t('restore.noRepositoriesTitle')}
		description={t('restore.noRepositoriesDescription')}
		bordered
	>
		<Button variant="primary" onclick={() => goto(resolve('/(app)/repositories'))}>
			{t('restore.noRepositoriesAction')}
		</Button>
	</EmptyState>
{:else}
	{#if error}
		<Alert tone="danger" title={error} />
	{/if}
	{#if view?.state === 'failed'}
		<Alert tone="danger" title={t('restore.listFailed', { reason: reasonOf(view.error_code) })}>
			{#snippet actions()}
				{#if view?.job_id}
					<a class="alert-link" href={resolve('/(app)/jobs/[id]', { id: view.job_id })}>
						{t('restore.openJob')}
					</a>
				{/if}
			{/snippet}
		</Alert>
	{/if}
	{#if view?.truncated}
		<Alert tone="attention" title={t('restore.listTruncated')} />
	{/if}

	<div class="list">
		<Panel
			title={t('restore.snapshots')}
			subtitle={view?.state === 'done'
				? hasFilters(filters)
					? t('restore.listFiltered', { shown: matching.length, total: snapshots.length })
					: t('restore.listCount', { count: snapshots.length })
				: ''}
			flush
		>
			{#snippet actions()}
				{#if view?.state === 'done' && view.fetched_at}
					<span class="muted">{t('restore.listAge', { time: relativeTime(view.fetched_at) })}</span>
				{/if}
				<Button
					variant="ghost"
					size="sm"
					icon="refresh"
					aria-label={t('restore.refresh')}
					title={t('restore.refresh')}
					loading={pending}
					disabled={!repositoryId}
					onclick={() => void find()}
				/>
			{/snippet}
			{#if view?.state === 'done' && snapshots.length > 0}
				<SnapshotFilterBar {snapshots} {filters} {profiles} onchange={change} onreset={reset} />
			{/if}
			{#if loading || (pending && !view)}
				<div class="list__state">
					<p class="muted">{t('restore.listLoading')}</p>
					<Skeleton variant="row" />
				</div>
			{:else if view?.state === 'done' && snapshots.length === 0}
				<EmptyState
					icon="restore"
					title={t('restore.listNoSnapshots')}
					description={t('restore.listNoSnapshotsDescription')}
				/>
			{:else if view && matching.length === 0}
				<EmptyState icon="search" title={t('restore.listEmpty')}>
					<Button onclick={reset}>{t('restore.reset')}</Button>
				</EmptyState>
			{:else}
				<div class="head" aria-hidden="true">
					<span>{t('restore.snapshot')}</span>
					<span>{t('restore.time')}</span>
					<span>{t('restore.hostColumn')}</span>
					<span>{t('restore.tags')}</span>
					<span class="head__size">{t('restore.size')}</span>
				</div>
				<ul class="rows">
					{#each shown as row (row.id)}
						<li class="row" class:row--opened={row.id === opened}>
							<span class="row__snapshot">
								<!-- eslint-disable svelte/no-navigation-without-resolve -- the address starts with a resolved path -->
								<a
									class="row__id"
									href={detailHref(row)}
									aria-label={t('restore.openSnapshot', { id: row.short_id })}
									aria-current={row.id === opened ? 'true' : undefined}>{row.short_id}</a
								>
								<!-- eslint-enable svelte/no-navigation-without-resolve -->
								<span class="row__paths">{row.paths.join(', ')}</span>
							</span>
							<span class="row__time">{row.time ? formatLocalTime(row.time) : '—'}</span>
							<span class="row__host">{row.hostname}</span>
							<span class="row__tags"><SnapshotTags tags={row.tags} {profiles} /></span>
							<span class="row__size">
								{row.total_bytes == null ? '—' : formatBytes(row.total_bytes)}
							</span>
						</li>
					{/each}
				</ul>
				{#if matching.length > shown.length}
					<div class="more">
						<Button onclick={() => (visible += PAGE_SIZE)}>
							{t('restore.showMore', { count: matching.length - shown.length })}
						</Button>
					</div>
				{/if}
			{/if}
		</Panel>
	</div>
{/if}

<style>
	.repository {
		display: flex;
		align-items: center;
		gap: var(--rh-space-2);
	}

	.repository__label {
		color: var(--rh-color-text-muted);
		white-space: nowrap;
	}

	.repository :global(.rh-select) {
		min-width: 12rem;
	}

	.list {
		margin-top: var(--rh-space-4);
	}

	.list__state {
		padding: var(--rh-space-4);
	}

	.muted {
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.alert-link {
		color: var(--rh-color-link);
		font-weight: var(--rh-font-weight-semibold);
	}

	.head,
	.row {
		display: grid;
		grid-template-columns: minmax(9rem, 1.2fr) 10rem 9rem minmax(12rem, 3fr) 6rem;
		gap: var(--rh-space-3);
		align-items: start;
		padding: var(--rh-space-3) var(--rh-space-4);
	}

	.head {
		border-bottom: var(--rh-border-width) solid var(--rh-color-border);
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
		font-weight: var(--rh-font-weight-semibold);
	}

	.head__size,
	.row__size {
		text-align: right;
		font-variant-numeric: tabular-nums;
	}

	.rows {
		margin: 0;
		padding: 0;
		list-style: none;
	}

	.row {
		position: relative;
		border-bottom: var(--rh-border-width) solid var(--rh-color-border-muted);
	}

	.row:last-child {
		border-bottom: 0;
	}

	.row:hover {
		background-color: var(--rh-color-hover);
	}

	.row--opened,
	.row--opened:hover {
		background-color: var(--rh-color-selected);
		box-shadow: inset 0 0 0 1px var(--rh-color-link);
	}

	.row__snapshot {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-1);
		min-width: 0;
	}

	.row__id {
		align-self: flex-start;
		color: var(--rh-color-link);
		font-family: var(--rh-font-mono, monospace);
		font-weight: var(--rh-font-weight-semibold);
		text-decoration: none;
	}

	/* The link covers the whole row, so a click anywhere on it opens the snapshot. */
	.row__id::after {
		content: '';
		position: absolute;
		inset: 0;
	}

	.row__id:hover {
		text-decoration: underline;
	}

	.row__id:focus-visible {
		outline: none;
	}

	.row__id:focus-visible::after {
		outline: 2px solid var(--rh-color-focus, var(--rh-color-link));
		outline-offset: -2px;
	}

	.row__paths {
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
		overflow-wrap: anywhere;
	}

	.row__time {
		color: var(--rh-color-text-muted);
	}

	.row__host,
	.row__time {
		overflow-wrap: anywhere;
	}

	.more {
		display: flex;
		justify-content: center;
		padding: var(--rh-space-3);
		border-top: var(--rh-border-width) solid var(--rh-color-border-muted);
	}

	@media (max-width: 40rem) {
		/* The label leaves the screen but stays the accessible name; the header has no room for both. */
		.repository__label {
			position: absolute;
			width: 1px;
			height: 1px;
			overflow: hidden;
			clip-path: inset(50%);
		}

		.repository :global(.rh-select) {
			min-width: 9rem;
		}
	}

	@media (max-width: 52rem) {
		.head {
			display: none;
		}

		.row {
			grid-template-columns: minmax(0, 1fr) auto;
			grid-template-areas:
				'snapshot size'
				'time host'
				'tags tags';
			gap: var(--rh-space-1) var(--rh-space-3);
		}

		.row__snapshot {
			grid-area: snapshot;
		}

		.row__size {
			grid-area: size;
		}

		.row__time {
			grid-area: time;
			font-size: var(--rh-font-size-sm);
		}

		.row__host {
			grid-area: host;
			font-size: var(--rh-font-size-sm);
			text-align: right;
		}

		.row__tags {
			grid-area: tags;
		}
	}
</style>
