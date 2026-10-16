<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { ApiError, service } from '#lib/api/index.ts';
	import type { Repository, SnapshotItem, SnapshotsView } from '#lib/api/index.ts';
	import SnapshotTags from '#lib/components/SnapshotTags.svelte';
	import {
		Alert,
		Button,
		EmptyState,
		PageHeader,
		Panel,
		Select,
		Skeleton,
		TextField
	} from '#lib/components/ui/index.ts';
	import type { SelectOption } from '#lib/components/ui/index.ts';
	import { formatBytes, formatLocalTime, relativeTime } from '#lib/format.ts';
	import { t, tErrorCode, tOr } from '#lib/i18n/index.ts';
	import { EMPTY_FILTERS, filtersFromQuery, filtersToQuery, targetExtra } from '#lib/restore.ts';
	import { BrowseTimeout, settle } from '#lib/restore-poll.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';

	const PAGE_SIZE = 15;

	const query = page.url.searchParams;
	const initial = filtersFromQuery(query);
	// The snapshot that the person had open: the detail page links back with it.
	const opened = query.get('snapshot') ?? '';

	let repositories = $state<Repository[]>([]);
	let profiles = $state(new Map<string, string>());
	let repositoryId = $state(query.get('repository') ?? '');
	let host = $state(initial.host);
	let tag = $state(initial.tag);
	let path = $state(initial.path);
	let from = $state(initial.from);
	let to = $state(initial.to);

	let loading = $state(true);
	let pending = $state(false);
	let error = $state<string | null>(null);
	let view = $state<SnapshotsView | null>(null);
	let visible = $state(PAGE_SIZE);
	// A newer request makes the answer of an older one obsolete.
	let generation = 0;

	const canRestore = $derived(['admin', 'operator'].includes(getSession()?.role ?? ''));
	const repositoryOptions = $derived<SelectOption[]>(
		repositories.map((item) => ({ value: item.id, label: item.name }))
	);
	const snapshots = $derived(view?.snapshots ?? []);
	const shown = $derived(snapshots.slice(0, visible));

	function describe(problem: unknown): string {
		if (problem instanceof ApiError) return tErrorCode(problem.code) ?? problem.title;
		if (problem instanceof BrowseTimeout) return problem.message;
		return t('table.error');
	}

	function reasonOf(code: string | null | undefined): string {
		if (!code) return '';
		return tOr(`jobs.reason.${code}`, code);
	}

	// The date inputs hold a day. The range covers the whole local day.
	function range(): { from?: string; to?: string } {
		return {
			from: from ? new Date(`${from}T00:00:00`).toISOString() : undefined,
			to: to ? new Date(`${to}T23:59:59.999`).toISOString() : undefined
		};
	}

	function filters() {
		return { host: host.trim(), tag: tag.trim(), path: path.trim(), from, to };
	}

	function sameList(list: string[], value: string): boolean {
		return value === '' ? list.length === 0 : list.length === 1 && list[0] === value;
	}

	// The stored list answers the filters of the address only when restic applied the same ones.
	function answers(stored: SnapshotsView): boolean {
		const wanted = filters();
		return (
			sameList(stored.hosts, wanted.host) &&
			sameList(stored.tags, wanted.tag) &&
			sameList(stored.paths, wanted.path)
		);
	}

	function remember(): void {
		const address = `${resolve('/(app)/restore')}?${filtersToQuery(filters(), { repository: repositoryId, ...targetExtra(query) })}`;
		// eslint-disable-next-line svelte/no-navigation-without-resolve -- the address starts with a resolved path
		void goto(address, { replaceState: true });
	}

	async function show(next: SnapshotsView, mine: number): Promise<void> {
		const done = await settle(
			next,
			(item) => service.getSnapshots(repositoryId, { job: item.job_id ?? undefined, ...range() }),
			{ cancelled: () => mine !== generation }
		);
		if (done && mine === generation) {
			view = done;
			visible = PAGE_SIZE;
		}
	}

	/** Reads the stored list of the repository. A repository without a fitting one gets a request. */
	async function open(): Promise<void> {
		generation += 1;
		const mine = generation;
		view = null;
		error = null;
		if (!repositoryId) return;
		try {
			const stored = await service.getSnapshots(repositoryId, range());
			if (mine !== generation) return;
			if ((stored.state === 'empty' || !answers(stored)) && canRestore) await find();
			else {
				view = stored;
				visible = PAGE_SIZE;
			}
		} catch (problem) {
			if (mine === generation) error = describe(problem);
		}
	}

	async function find(): Promise<void> {
		generation += 1;
		const mine = generation;
		pending = true;
		error = null;
		try {
			const wanted = filters();
			const list = (value: string) => (value ? [value] : []);
			const started = await service.startSnapshots(repositoryId, {
				hosts: list(wanted.host),
				tags: list(wanted.tag),
				paths: list(wanted.path)
			});
			if (mine !== generation) return;
			view = started;
			remember();
			await show(started, mine);
		} catch (problem) {
			if (mine === generation) error = describe(problem);
		} finally {
			if (mine === generation) pending = false;
		}
	}

	function reset(): void {
		host = EMPTY_FILTERS.host;
		tag = EMPTY_FILTERS.tag;
		path = EMPTY_FILTERS.path;
		from = EMPTY_FILTERS.from;
		to = EMPTY_FILTERS.to;
		void find();
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
		return `${address}?${filtersToQuery(filters(), targetExtra(query))}`;
	}

	onMount(async () => {
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
	});
</script>

<svelte:head>
	<title>{t('restore.title')} · {t('app.name')}</title>
</svelte:head>

<PageHeader title={t('restore.title')} description={t('restore.description')} />

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
	<Panel title={t('restore.filters')}>
		<form
			class="filters"
			onsubmit={(event) => {
				event.preventDefault();
				void find();
			}}
		>
			<Select
				id="restore-repository"
				label={t('restore.repository')}
				options={repositoryOptions}
				bind:value={repositoryId}
				onchange={changeRepository}
			/>
			<TextField id="restore-host" label={t('restore.host')} bind:value={host} />
			<TextField id="restore-tag" label={t('restore.tag')} bind:value={tag} />
			<TextField id="restore-path" label={t('restore.path')} mono bind:value={path} />
			<TextField id="restore-from" type="date" label={t('restore.from')} bind:value={from} />
			<TextField id="restore-to" type="date" label={t('restore.to')} bind:value={to} />
			<div class="filters__actions">
				<Button type="submit" variant="primary" icon="search" loading={pending}>
					{t('restore.find')}
				</Button>
				<Button variant="ghost" onclick={reset}>{t('restore.reset')}</Button>
			</div>
		</form>
	</Panel>

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
			subtitle={view?.state === 'done' ? t('restore.listCount', { count: snapshots.length }) : ''}
			flush
		>
			{#snippet actions()}
				{#if view?.state === 'done' && view.fetched_at}
					<span class="muted">{t('restore.listAge', { time: relativeTime(view.fetched_at) })}</span>
				{/if}
			{/snippet}
			{#if loading || (pending && snapshots.length === 0)}
				<div class="list__state"><Skeleton variant="row" /></div>
			{:else if snapshots.length === 0}
				<p class="list__state muted">
					{view?.state === 'empty' ? t('restore.listNone') : t('restore.listEmpty')}
				</p>
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
				{#if snapshots.length > shown.length}
					<div class="more">
						<Button onclick={() => (visible += PAGE_SIZE)}>
							{t('restore.showMore', { count: snapshots.length - shown.length })}
						</Button>
					</div>
				{/if}
			{/if}
		</Panel>
	</div>
{/if}

<style>
	.filters {
		display: grid;
		grid-template-columns: repeat(auto-fit, minmax(10rem, 1fr));
		gap: var(--rh-space-3);
		align-items: end;
	}

	.filters__actions {
		display: flex;
		grid-column: 1 / -1;
		align-items: center;
		gap: var(--rh-space-2);
	}

	.list {
		margin-top: var(--rh-space-4);
	}

	.list__state {
		margin: 0;
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
