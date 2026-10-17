<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { ApiError, service } from '#lib/api/index.ts';
	import type { Agent, Repository, SnapshotItem, SnapshotsView } from '#lib/api/index.ts';
	import RestoreDialog from '#lib/components/RestoreDialog.svelte';
	import { Page } from '#lib/components/layout/index.ts';
	import {
		Alert,
		Button,
		EmptyState,
		FilterBar,
		ListView,
		Select,
		Skeleton
	} from '#lib/components/ui/index.ts';
	import type { FilterFacet, ListMeta, SelectOption } from '#lib/components/ui/index.ts';
	import { formatLocalTime, relativeTime } from '#lib/format.ts';
	import { dayHeading, size } from '#lib/list-format.ts';
	import { ENTITY_ICONS } from '#lib/vocabulary.ts';
	import { getLocale, t, tErrorCode, tOr } from '#lib/i18n/index.ts';
	import {
		EMPTY_FILTERS,
		TARGET_PARAM,
		filtersFromQuery,
		filtersToQuery,
		targetExtra
	} from '#lib/restore.ts';
	import type { RestoreFilters } from '#lib/restore.ts';
	import { BrowseTimeout, settle } from '#lib/restore-poll.ts';
	import { applyFilters, facetCounts, tagLabel } from '#lib/snapshot-filter.ts';
	import { loadRepository, pickRepository, saveRepository } from '#lib/restore-repository.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';

	const PAGE_SIZE = 25;
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
	let tablePage = $state(1);
	// A newer request makes the answer of an older one obsolete.
	let generation = 0;
	let addressTimer: ReturnType<typeof setTimeout> | undefined;

	const canRestore = $derived(['admin', 'operator'].includes(getSession()?.role ?? ''));
	const repositoryOptions = $derived<SelectOption[]>(
		repositories.map((item) => ({ value: item.id, label: item.name }))
	);
	const snapshots = $derived(view?.state === 'done' ? view.snapshots : []);
	const matching = $derived(
		[...applyFilters(snapshots, filters, Date.now())].sort(
			(left, right) => timeOf(right) - timeOf(left)
		)
	);

	function hoursMinutes(value: string): string {
		return new Intl.DateTimeFormat(getLocale(), {
			hour: '2-digit',
			minute: '2-digit',
			hour12: false
		}).format(new Date(value));
	}

	function meta(snapshot: SnapshotItem): ListMeta[] {
		const [first, ...rest] = snapshot.paths;
		return [
			snapshot.hostname,
			first ? { text: rest.length > 0 ? `${first} +${rest.length}` : first, mono: true } : null,
			{
				text: snapshot.tags.map((tag) => tagLabel(tag, profiles)).join(', '),
				title: snapshot.tags.join(', ')
			},
			{ text: snapshot.short_id, mono: true }
		];
	}

	let agents = $state<Agent[]>([]);
	let restoreOpen = $state(false);
	let restoreTarget = $state<SnapshotItem | null>(null);
	const restoreRepository = $derived(repositories.find((item) => item.id === repositoryId));

	async function openRestore(snapshot: SnapshotItem): Promise<void> {
		if (agents.length === 0) agents = await service.listAgents().catch(() => []);
		restoreTarget = snapshot;
		restoreOpen = true;
	}

	const facets = $derived<FilterFacet[]>([
		{
			key: 'host',
			label: t('restore.facetHost'),
			hint: t('restore.filterHint'),
			options: facetCounts(snapshots, (item) => [item.hostname])
		},
		{
			key: 'tag',
			label: t('restore.facetTag'),
			hint: t('restore.filterHint'),
			options: facetCounts(snapshots, (item) => item.tags).map((option) => ({
				...option,
				label: tagLabel(option.value, profiles)
			}))
		}
	]);

	function timeOf(snapshot: SnapshotItem): number {
		return snapshot.time ? Date.parse(snapshot.time) : Number.NEGATIVE_INFINITY;
	}

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
				tablePage = 1;
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
				tablePage = 1;
			}
		} catch (problem) {
			if (mine === generation) error = describe(problem);
		} finally {
			if (mine === generation) pending = false;
		}
	}

	function change(patch: Partial<RestoreFilters>): void {
		filters = { ...filters, ...patch };
		tablePage = 1;
		rememberSoon();
	}

	function reset(): void {
		filters = { ...EMPTY_FILTERS };
		tablePage = 1;
		rememberSoon();
	}

	function changeRepository(): void {
		saveRepository(getSession(), repositoryId);
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
				repositoryId = pickRepository(repositories, repositoryId, loadRepository(getSession()));
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

{#snippet refresh()}
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

<Page
	title={t('restore.title')}
	description={t('restore.description')}
	count={view?.state === 'done' ? snapshots.length : undefined}
>
	{#snippet context()}
		{#if canRestore && repositories.length > 0}
			<div class="repository">
				<label for="restore-repository">{t('restore.repositoryLabel')}</label>
				<Select
					id="restore-repository"
					options={repositoryOptions}
					bind:value={repositoryId}
					onchange={changeRepository}
				/>
			</div>
		{/if}
	{/snippet}

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
		{#if error || view?.state === 'failed' || view?.truncated}
			<div class="notices">
				{#if error}
					<Alert tone="danger" title={error}>
						{#snippet actions()}
							{@render refresh()}
						{/snippet}
					</Alert>
				{/if}
				{#if view?.state === 'failed'}
					<Alert
						tone="danger"
						title={t('restore.listFailed', { reason: reasonOf(view.error_code) })}
					>
						{#snippet actions()}
							{#if view?.job_id}
								<a class="alert-link" href={resolve('/(app)/jobs/[id]', { id: view.job_id })}>
									{t('restore.openJob')}
								</a>
							{/if}
							{@render refresh()}
						{/snippet}
					</Alert>
				{/if}
				{#if view?.truncated}
					<Alert tone="attention" title={t('restore.listTruncated')} />
				{/if}
			</div>
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
				bordered
			>
				<Button
					icon="refresh"
					loading={pending}
					disabled={!repositoryId}
					onclick={() => void find()}>{t('restore.refresh')}</Button
				>
			</EmptyState>
		{:else if view?.state === 'done'}
			<ListView
				label={t('restore.snapshots')}
				items={matching}
				itemKey={(row) => row.id}
				icon={() => ({ name: ENTITY_ICONS.snapshot })}
				title={(row) => (row.time ? hoursMinutes(row.time) : row.short_id)}
				titleLabel={(row) =>
					t('restore.openSnapshot', {
						id: row.time ? `${row.short_id}, ${formatLocalTime(row.time)}` : row.short_id
					})}
				href={detailHref}
				selected={(row) => row.id === opened}
				{meta}
				aside={(row) => (row.total_bytes == null ? '—' : size(row.total_bytes))}
				groupBy={(row) => (row.time ? dayHeading(row.time) : t('restore.noTime'))}
				actions={(row) => [
					{
						id: 'restore',
						label: t('restore.restoreSnapshot'),
						icon: 'restore',
						onSelect: () => void openRestore(row)
					}
				]}
				bind:page={tablePage}
				pageSize={PAGE_SIZE}
				filtered
				onReset={reset}
			>
				{#snippet toolbar()}
					<FilterBar
						label={t('restore.filterBar')}
						value={{ ...filters }}
						search={{
							key: 'path',
							id: 'restore-path',
							label: t('restore.pathSearch'),
							placeholder: t('restore.pathPlaceholder')
						}}
						total={matching.length}
						{facets}
						period
						onchange={change}
						onreset={reset}
					>
						{#snippet end()}
							{#if view?.fetched_at}
								<span class="muted"
									>{t('restore.listAge', { time: relativeTime(view.fetched_at) })}</span
								>
							{/if}
							{@render refresh()}
						{/snippet}
					</FilterBar>
				{/snippet}
			</ListView>
		{/if}
	{/if}

	{#if restoreRepository && restoreTarget}
		<RestoreDialog
			bind:open={restoreOpen}
			repository={restoreRepository}
			snapshot={restoreTarget}
			paths={[]}
			{agents}
			preferredAgentId={query.get(TARGET_PARAM) ?? ''}
		/>
	{/if}
</Page>

<style>
	.repository {
		display: flex;
		align-items: center;
		gap: var(--rh-space-2);
	}

	.repository :global(.rh-field) {
		flex: 1 1 auto;
	}

	.repository :global(.rh-select) {
		min-width: 12rem;
	}

	.notices {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-3);
		margin-bottom: var(--rh-space-4);
	}

	.list__state {
		padding: var(--rh-space-4);
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
	}

	.muted {
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.alert-link {
		color: var(--rh-color-link);
		font-weight: var(--rh-font-weight-semibold);
	}
</style>
