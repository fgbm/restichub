<script lang="ts">
	import { onMount } from 'svelte';
	import { coalesce } from '#lib/events.ts';
	import { isStreamConnected, onEvent } from '#lib/stores/events.svelte.ts';
	import { resolve } from '$app/paths';
	import { ApiError, service } from '#lib/api/index.ts';
	import type { Job, JobStatus } from '#lib/api/index.ts';
	import { Page } from '#lib/components/layout/index.ts';
	import {
		Button,
		ConfirmDialog,
		DataTable,
		EmptyState,
		FilterBar,
		StatusPill
	} from '#lib/components/ui/index.ts';
	import type {
		FilterFacet,
		RowAction,
		StatusTone,
		TableColumn
	} from '#lib/components/ui/index.ts';
	import { matchesSearch } from '#lib/filters.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import type { MessageKey } from '#lib/i18n/index.ts';
	import { formatDuration } from '#lib/format.ts';
	import { queryFilters } from '#lib/stores/query-filters.svelte.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';

	const STATUSES: JobStatus[] = [
		'queued',
		'sent',
		'running',
		'succeeded',
		'failed',
		'expired',
		'dead_letter'
	];
	const PAGE_SIZE = 50;

	let jobs = $state<Job[]>([]);
	let nextCursor = $state<string | null>(null);
	let loading = $state(true);
	let loadingMore = $state(false);
	let error = $state<string | null>(null);
	// The host page links here with the host to show (`?agent_id=`).
	const filters = queryFilters(['q', 'status', 'agent_id'] as const);
	const statusFilter = $derived(filters.value.status);
	const agentFilter = $derived(filters.value.agent_id || undefined);

	let cancelTarget = $state<Job | null>(null);
	let cancelOpen = $state(false);
	let cancelling = $state(false);

	const canWrite = $derived(['admin', 'operator'].includes(getSession()?.role ?? ''));

	const facets = $derived<FilterFacet[]>([
		{
			key: 'status',
			label: t('jobs.status'),
			multiple: false,
			options: STATUSES.map((value) => ({ value, label: t(`jobs.status.${value}` as MessageKey) }))
		},
		{
			key: 'agent_id',
			label: t('jobs.host'),
			multiple: false,
			options: [
				...new Map(
					jobs
						.filter((job) => job.agent_id)
						.map((job) => [job.agent_id, job.agent_hostname ?? job.agent_id])
				)
			].map(([value, label]) => ({ value, label }))
		}
	]);

	const columns = $derived<TableColumn<Job>[]>([
		{
			key: 'status',
			header: t('jobs.status'),
			nowrap: true,
			value: statusLabel,
			render: statusCell
		},
		{ key: 'plan', header: t('jobs.plan'), value: planLabel, truncate: 'sm', primary: true },
		{
			key: 'kind',
			header: t('jobDetail.kind'),
			nowrap: true,
			value: (row) => t(`jobs.kind.${row.kind}` as MessageKey)
		},
		{
			key: 'host',
			header: t('jobs.host'),
			truncate: 'sm',
			value: (row) => row.agent_hostname ?? t('common.none')
		},
		{
			key: 'repository',
			header: t('jobs.repository'),
			truncate: 'sm',
			value: (row) => row.repository_name ?? ''
		},
		{
			key: 'started',
			header: t('jobs.started'),
			date: 'absolute',
			value: (row) => row.started_at ?? ''
		},
		{
			key: 'duration',
			nowrap: true,
			align: 'right',
			header: t('jobs.duration'),
			value: (row) => durationOf(row) || '—'
		}
	]);

	const shown = $derived(
		jobs.filter((job) =>
			matchesSearch(filters.value.q, [
				planLabel(job),
				job.agent_hostname,
				job.repository_name,
				job.id
			])
		)
	);

	const empty = $derived(
		!loading &&
			!error &&
			jobs.length === 0 &&
			statusFilter === '' &&
			!agentFilter &&
			!filters.value.q
	);

	// A cancelled job ends as `failed` with the result `cancelled`; the screen shows it apart.
	function isCancelled(job: Job): boolean {
		return job.status === 'failed' && job.result_code === 'cancelled';
	}

	function hasWarnings(job: Job): boolean {
		return job.status === 'succeeded' && job.result_code === 'succeeded_with_warnings';
	}

	function statusLabel(job: Job): string {
		if (hasWarnings(job)) return t('jobs.status.succeeded_with_warnings');
		return t(`jobs.status.${isCancelled(job) ? 'cancelled' : job.status}` as MessageKey);
	}

	function toneOf(job: Job): StatusTone {
		if (isCancelled(job)) return 'cancelled';
		if (hasWarnings(job)) return 'warning';
		switch (job.status) {
			case 'queued':
				return 'queued';
			case 'sent':
				return 'info';
			case 'running':
				return 'running';
			case 'succeeded':
				return 'success';
			case 'expired':
				return 'warning';
			default:
				return 'failed';
		}
	}

	function planLabel(job: Job): string {
		return job.plan_name ?? t(`jobs.kind.${job.kind}` as MessageKey);
	}

	function durationOf(job: Job): string {
		if (!job.started_at) return '';
		return formatDuration(job.started_at, job.finished_at ?? new Date().toISOString());
	}

	function cancellable(job: Job): boolean {
		return job.status === 'queued' || job.status === 'sent' || job.status === 'running';
	}

	function actionsFor(job: Job): RowAction[] {
		if (!canWrite || !cancellable(job)) return [];
		return [
			{
				id: 'cancel',
				label: t('jobs.cancel'),
				icon: 'stop',
				danger: true,
				onSelect: () => {
					cancelTarget = job;
					cancelOpen = true;
				}
			}
		];
	}

	function messageOf(problem: unknown): string {
		if (problem instanceof ApiError) return tErrorCode(problem.code) ?? problem.title;
		return t('table.error');
	}

	async function load(silent = false): Promise<void> {
		if (!silent) {
			loading = true;
			extended = false;
		}
		error = null;
		try {
			const page = await service.listJobs({
				limit: PAGE_SIZE,
				status: statusFilter ? (statusFilter as JobStatus) : undefined,
				agent_id: agentFilter
			});
			jobs = page.items;
			nextCursor = page.next_cursor ?? null;
		} catch (problem) {
			error = messageOf(problem);
		} finally {
			loading = false;
		}
	}

	async function loadMore(): Promise<void> {
		if (!nextCursor) return;
		loadingMore = true;
		try {
			const page = await service.listJobs({
				limit: PAGE_SIZE,
				status: statusFilter ? (statusFilter as JobStatus) : undefined,
				agent_id: agentFilter,
				cursor: nextCursor
			});
			jobs = [...jobs, ...page.items];
			nextCursor = page.next_cursor ?? null;
			extended = true;
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		} finally {
			loadingMore = false;
		}
	}

	let loadedFilter: string | undefined;
	onMount(() => {
		loadedFilter = `${statusFilter}|${agentFilter ?? ''}`;
		void load();
	});

	// The status and the host filter on the server; the search narrows the loaded rows.
	$effect(() => {
		const key = `${statusFilter}|${agentFilter ?? ''}`;
		if (loadedFilter === undefined || key === loadedFilter) return;
		loadedFilter = key;
		void load();
	});

	// A page that the user extended with "load more" keeps its rows until the next full load.
	let extended = false;
	async function reload(): Promise<void> {
		if (extended || loading || loadingMore) return;
		await load(true);
	}

	const reloadSoon = coalesce(reload, 300);
	onMount(() =>
		onEvent((event) => {
			if (event.type === 'job.status' || event.type === 'resync') reloadSoon();
		})
	);

	// The stream pushes the changes. The poll covers a stream that is down.
	$effect(() => {
		const timer = setInterval(
			() => void (!document.hidden && reload()),
			isStreamConnected() ? 60_000 : 15_000
		);
		return () => clearInterval(timer);
	});

	async function confirmCancel(): Promise<void> {
		if (!cancelTarget) return;
		cancelling = true;
		try {
			const job = await service.cancelJob(cancelTarget.id);
			pushToast(
				'success',
				t(job.status === 'queued' || isCancelled(job) ? 'jobs.cancelled' : 'jobs.cancelRequested')
			);
			cancelOpen = false;
			cancelTarget = null;
			await load();
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		} finally {
			cancelling = false;
		}
	}
</script>

{#snippet statusCell(job: Job)}
	<StatusPill tone={toneOf(job)}>{statusLabel(job)}</StatusPill>
{/snippet}

<Page
	title={t('jobs.title')}
	description={t('jobs.description')}
	count={loading ? undefined : jobs.length}
>
	{#snippet actions()}
		<Button icon="refresh" onclick={() => load()}>{t('common.refresh')}</Button>
	{/snippet}

	{#if empty}
		<EmptyState
			icon="job"
			title={t('jobs.emptyTitle')}
			description={t('jobs.emptyDescription')}
			bordered
		/>
	{:else}
		<DataTable
			{columns}
			rows={shown}
			{loading}
			{error}
			rowKey={(job) => job.id}
			rowHref={(job) => resolve('/(app)/jobs/[id]', { id: job.id })}
			paginated={false}
			actions={actionsFor}
			moreLabel={(job) => t('jobs.moreActions', { name: planLabel(job) })}
			emptyTitle={t('filter.noMatches')}
			emptyDescription={t('filter.noMatchesDescription')}
		>
			{#snippet toolbar()}
				<FilterBar
					label={t('jobs.filters')}
					value={filters.value}
					search={{ key: 'q', label: t('hosts.search'), placeholder: t('jobs.searchPlaceholder') }}
					{facets}
					onchange={filters.update}
					onreset={filters.reset}
				/>
			{/snippet}
		</DataTable>
		{#if nextCursor}
			<div class="more">
				<Button loading={loadingMore} onclick={loadMore}>{t('jobs.loadMore')}</Button>
			</div>
		{/if}
	{/if}

	<ConfirmDialog
		bind:open={cancelOpen}
		title={t('jobs.cancelTitle')}
		message={t('jobs.cancelBody', { name: cancelTarget ? planLabel(cancelTarget) : '' })}
		confirmLabel={t('jobs.cancel')}
		cancelLabel={t('jobs.cancelKeep')}
		tone="danger"
		loading={cancelling}
		onConfirm={confirmCancel}
	/>
</Page>

<style>
	.more {
		display: flex;
		justify-content: center;
		margin-top: var(--rh-space-4);
	}
</style>
