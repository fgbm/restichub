<script lang="ts">
	import { onMount } from 'svelte';
	import { ApiError, service } from '#lib/api/index.ts';
	import type { Agent, Repository, SnapshotItem, SnapshotsView } from '#lib/api/index.ts';
	import RestoreDialog from '#lib/components/RestoreDialog.svelte';
	import SnapshotTree from '#lib/components/SnapshotTree.svelte';
	import {
		Alert,
		Badge,
		Button,
		EmptyState,
		Icon,
		PageHeader,
		Panel,
		Select,
		Table,
		TextField
	} from '#lib/components/ui/index.ts';
	import type { SelectOption, TableColumn } from '#lib/components/ui/index.ts';
	import { formatBytes, formatLocalTime, relativeTime } from '#lib/format.ts';
	import { t, tErrorCode, tOr } from '#lib/i18n/index.ts';
	import { includePaths } from '#lib/restore.ts';
	import { BrowseTimeout, settle } from '#lib/restore-poll.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';

	let repositories = $state<Repository[]>([]);
	let agents = $state<Agent[]>([]);
	let repositoryId = $state('');
	let host = $state('');
	let tag = $state('');
	let path = $state('');
	let from = $state('');
	let to = $state('');

	let loading = $state(true);
	let pending = $state(false);
	let error = $state<string | null>(null);
	let view = $state<SnapshotsView | null>(null);
	let chosen = $state<SnapshotItem | null>(null);
	let selection = $state(new Set<string>());
	let dialogOpen = $state(false);
	// A newer request makes the answer of an older one obsolete.
	let generation = 0;

	const canRestore = $derived(['admin', 'operator'].includes(getSession()?.role ?? ''));
	const repository = $derived(repositories.find((item) => item.id === repositoryId) ?? null);
	const repositoryOptions = $derived<SelectOption[]>(
		repositories.map((item) => ({ value: item.id, label: item.name }))
	);
	const snapshots = $derived(view?.snapshots ?? []);
	const paths = $derived(includePaths(selection));

	const columns = $derived<TableColumn<SnapshotItem>[]>([
		{
			key: 'snapshot',
			header: t('restore.snapshot'),
			value: (row) => row.id,
			render: snapshotCell
		},
		{
			key: 'time',
			nowrap: true,
			header: t('restore.time'),
			value: (row) => row.time ?? '',
			render: timeCell
		},
		{ key: 'host', header: t('restore.hostColumn'), value: (row) => row.hostname },
		{
			key: 'tags',
			header: t('restore.tags'),
			value: (row) => row.tags.join(' '),
			render: tagsCell
		},
		{
			key: 'size',
			header: t('restore.size'),
			align: 'right',
			value: (row) => String(row.total_bytes ?? 0),
			cell: (row) => (row.total_bytes == null ? '—' : formatBytes(row.total_bytes))
		}
	]);

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

	function keep(snapshot: SnapshotItem | null, next: SnapshotsView): void {
		view = next;
		if (snapshot && !next.snapshots.some((item) => item.id === snapshot.id)) {
			chosen = null;
			selection = new Set();
		}
	}

	async function show(next: SnapshotsView, mine: number): Promise<void> {
		const done = await settle(
			next,
			(item) => service.getSnapshots(repositoryId, { job: item.job_id ?? undefined, ...range() }),
			{ cancelled: () => mine !== generation }
		);
		if (done && mine === generation) keep(chosen, done);
	}

	/** Reads the stored list of the repository. A repository without one gets a first request. */
	async function open(): Promise<void> {
		generation += 1;
		const mine = generation;
		view = null;
		chosen = null;
		selection = new Set();
		error = null;
		if (!repositoryId) return;
		try {
			const stored = await service.getSnapshots(repositoryId, range());
			if (mine !== generation) return;
			if (stored.state === 'empty' && canRestore) await find();
			else keep(null, stored);
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
			const filters = (value: string) => (value.trim() ? [value.trim()] : []);
			const started = await service.startSnapshots(repositoryId, {
				hosts: filters(host),
				tags: filters(tag),
				paths: filters(path)
			});
			if (mine !== generation) return;
			view = started;
			await show(started, mine);
		} catch (problem) {
			if (mine === generation) error = describe(problem);
		} finally {
			if (mine === generation) pending = false;
		}
	}

	async function reapplyDates(): Promise<void> {
		if (!view?.job_id) return;
		try {
			keep(chosen, await service.getSnapshots(repositoryId, { job: view.job_id, ...range() }));
		} catch (problem) {
			error = describe(problem);
		}
	}

	function choose(snapshot: SnapshotItem): void {
		if (chosen?.id === snapshot.id) return;
		chosen = snapshot;
		selection = new Set();
	}

	onMount(async () => {
		try {
			const [allRepositories, allAgents] = await Promise.all([
				service.listRepositories(),
				service.listAgents()
			]);
			repositories = allRepositories.filter((item) => item.initialized);
			agents = allAgents;
			repositoryId = repositories[0]?.id ?? '';
		} catch (problem) {
			error = describe(problem);
		} finally {
			loading = false;
		}
		await open();
	});
</script>

{#snippet snapshotCell(row: SnapshotItem)}
	<span class="snapshot">
		<span class="snapshot__head">
			<button
				type="button"
				class="snapshot__id"
				class:snapshot__id--chosen={chosen?.id === row.id}
				aria-pressed={chosen?.id === row.id}
				aria-label={t('restore.openSnapshot', { id: row.short_id })}
				onclick={() => choose(row)}>{row.short_id}</button
			>
			{#if chosen?.id === row.id}
				<Icon name="check" size="xs" />
			{/if}
		</span>
		<span class="snapshot__paths">{row.paths.join(', ')}</span>
	</span>
{/snippet}

{#snippet timeCell(row: SnapshotItem)}
	<span class="muted">{row.time ? formatLocalTime(row.time) : '—'}</span>
{/snippet}

{#snippet tagsCell(row: SnapshotItem)}
	<span class="tags">
		{#each row.tags as value (value)}
			<Badge>{value}</Badge>
		{/each}
	</span>
{/snippet}

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
	/>
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
				onchange={open}
			/>
			<TextField id="restore-host" label={t('restore.host')} bind:value={host} />
			<TextField id="restore-tag" label={t('restore.tag')} bind:value={tag} />
			<TextField id="restore-path" label={t('restore.path')} mono bind:value={path} />
			<TextField
				id="restore-from"
				type="date"
				label={t('restore.from')}
				bind:value={from}
				onchange={reapplyDates}
			/>
			<TextField
				id="restore-to"
				type="date"
				label={t('restore.to')}
				bind:value={to}
				onchange={reapplyDates}
			/>
			<div class="filters__action">
				<Button type="submit" variant="primary" icon="search" loading={pending}>
					{t('restore.find')}
				</Button>
			</div>
		</form>
	</Panel>

	{#if error}
		<Alert tone="danger" title={error} />
	{/if}
	{#if view?.state === 'failed'}
		<Alert tone="danger" title={t('restore.listFailed', { reason: reasonOf(view.error_code) })} />
	{/if}
	{#if view?.truncated}
		<Alert tone="attention" title={t('restore.listTruncated')} />
	{/if}

	<div class="layout">
		<Panel title={t('restore.snapshots')} flush>
			{#snippet actions()}
				{#if view?.state === 'done' && view.fetched_at}
					<span class="muted">{t('restore.listAge', { time: relativeTime(view.fetched_at) })}</span>
				{/if}
			{/snippet}
			<Table
				{columns}
				rows={snapshots}
				rowKey={(row) => row.id}
				loading={loading || (pending && snapshots.length === 0)}
				filterable={false}
				pageSize={6}
				onRowClick={choose}
				emptyTitle={view?.state === 'empty' ? t('restore.listNone') : t('restore.listEmpty')}
			/>
		</Panel>

		<Panel
			title={t('restore.files')}
			subtitle={chosen
				? t('restore.filesHint', { id: chosen.short_id, host: chosen.hostname })
				: undefined}
		>
			{#if chosen && repository}
				<SnapshotTree repositoryId={repository.id} snapshot={chosen} bind:selected={selection} />
			{:else}
				<p class="muted">{t('restore.pickSnapshot')}</p>
			{/if}
			{#snippet footer()}
				<div class="selection">
					<span class="muted">
						{paths.length === 0
							? t('restore.selectionNone')
							: t('restore.selectionSome', { count: paths.length })}
					</span>
					<span class="selection__actions">
						{#if paths.length > 0}
							<Button size="sm" variant="ghost" onclick={() => (selection = new Set())}>
								{t('restore.clearSelection')}
							</Button>
						{/if}
						<Button
							variant="primary"
							icon="restore"
							disabled={!chosen}
							onclick={() => (dialogOpen = true)}
						>
							{t('restore.restore')}
						</Button>
					</span>
				</div>
			{/snippet}
		</Panel>
	</div>

	{#if repository && chosen}
		<RestoreDialog bind:open={dialogOpen} {repository} snapshot={chosen} {paths} {agents} />
	{/if}
{/if}

<style>
	.filters {
		display: grid;
		grid-template-columns: repeat(auto-fit, minmax(10rem, 1fr));
		gap: var(--rh-space-3);
		align-items: end;
	}

	.filters__action {
		display: flex;
		align-items: end;
	}

	.layout {
		display: grid;
		grid-template-columns: minmax(0, 1.15fr) minmax(0, 1fr);
		gap: var(--rh-space-4);
		margin-top: var(--rh-space-4);
		align-items: start;
	}

	@media (max-width: 70rem) {
		.layout {
			grid-template-columns: minmax(0, 1fr);
		}
	}

	.snapshot {
		display: flex;
		flex-direction: column;
		align-items: flex-start;
		gap: var(--rh-space-1);
	}

	.snapshot__head {
		display: inline-flex;
		align-items: center;
		gap: var(--rh-space-2);
	}

	.snapshot__id {
		padding: 0;
		border: 0;
		background: none;
		color: var(--rh-color-link);
		font-family: var(--rh-font-mono, monospace);
		font-weight: var(--rh-font-weight-semibold);
		cursor: pointer;
	}

	.snapshot__id--chosen {
		color: var(--rh-color-text);
		text-decoration: underline;
	}

	.snapshot__paths {
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.tags {
		display: inline-flex;
		flex-wrap: wrap;
		gap: var(--rh-space-1);
	}

	.muted {
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.selection {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: var(--rh-space-3);
	}

	.selection__actions {
		display: inline-flex;
		align-items: center;
		gap: var(--rh-space-2);
	}
</style>
