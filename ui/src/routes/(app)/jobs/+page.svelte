<script lang="ts">
	import { onMount } from 'svelte';
	import { coalesce } from '#lib/events.ts';
	import { isStreamConnected, onEvent } from '#lib/stores/events.svelte.ts';
	import { resolve } from '$app/paths';
	import { ApiError, service } from '#lib/api/index.ts';
	import type { Job, JobKind, JobQuery, JobStatus } from '#lib/api/index.ts';
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
		TableColumn,
		TableSort
	} from '#lib/components/ui/index.ts';
	import { periodWindow } from '#lib/filters.ts';
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
	// The kinds that a person starts or a plan runs; the browse jobs of Restore stay out.
	const KINDS: JobKind[] = [
		'backup',
		'check',
		'forget',
		'prune',
		'restore',
		'init',
		'test_access',
		'unlock',
		'unlock_all'
	];
	const PAGE_SIZE = 25;

	let jobs = $state<Job[]>([]);
	let total = $state(0);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let hosts = $state<{ id: string; hostname: string }[]>([]);
	let repositories = $state<{ id: string; name: string }[]>([]);
	let plans = $state<{ id: string; name: string }[]>([]);
	// The list filters, sorts and pages on the server (FR-UI-01). The host page links here with the
	// host to show (`?agent_id=`).
	const filters = queryFilters([
		'agent_id',
		'repository_id',
		'plan_id',
		'kind',
		'status',
		'period',
		'from',
		'to',
		'page',
		'sort',
		'order'
	] as const);
	const SORT_KEYS: Record<string, NonNullable<JobQuery['sort']>> = {
		status: 'status',
		plan: 'plan',
		host: 'host',
		started: 'started_at'
	};
	const tablePage = $derived(Math.max(1, Number(filters.value.page) || 1));
	const tableSort = $derived<TableSort | undefined>(
		filters.value.sort
			? { key: filters.value.sort, direction: filters.value.order === 'asc' ? 'asc' : 'desc' }
			: undefined
	);

	let cancelTarget = $state<Job | null>(null);
	let cancelOpen = $state(false);
	let cancelling = $state(false);

	const canWrite = $derived(['admin', 'operator'].includes(getSession()?.role ?? ''));

	function queryOf(): JobQuery {
		const value = filters.value;
		const window = periodWindow(value, Date.now());
		const iso = (time: number) =>
			Number.isFinite(time) ? new Date(time).toISOString() : undefined;
		return {
			agent_id: value.agent_id || undefined,
			repository_id: value.repository_id || undefined,
			plan_id: value.plan_id || undefined,
			kind: value.kind || undefined,
			status: value.status ? (value.status as JobStatus) : undefined,
			from: iso(window.start),
			to: iso(window.end),
			sort: SORT_KEYS[value.sort],
			order: value.sort ? (value.order === 'asc' ? 'asc' : 'desc') : undefined,
			offset: (tablePage - 1) * PAGE_SIZE,
			limit: PAGE_SIZE
		};
	}

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
			options: hosts.map((host) => ({ value: host.id, label: host.hostname }))
		},
		{
			key: 'repository_id',
			label: t('jobs.repository'),
			options: repositories.map((repository) => ({ value: repository.id, label: repository.name }))
		},
		{
			key: 'plan_id',
			label: t('jobs.plan'),
			options: plans.map((plan) => ({ value: plan.id, label: plan.name }))
		},
		{
			key: 'kind',
			label: t('jobDetail.kind'),
			options: KINDS.map((value) => ({ value, label: t(`jobs.kind.${value}` as MessageKey) }))
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
			sortable: false,
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
			sortable: false,
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
			sortable: false,
			align: 'right',
			header: t('jobs.duration'),
			value: (row) => durationOf(row) || '—'
		}
	]);

	const empty = $derived(
		!loading && !error && total === 0 && !Object.values(filters.value).some(Boolean)
	);

	// A cancelled job ends as `failed` with the result `cancelled`; the screen shows it apart.
	function isCancelled(job: Job): boolean {
		return job.status === 'failed' && job.result_code === 'cancelled';
	}

	function hasWarnings(job: Job): boolean {
		return job.status === 'succeeded' && job.result_code === 'succeeded_with_warnings';
	}

	function fullLabel(job: Job): string | undefined {
		return hasWarnings(job) ? t('jobs.statusFull.succeeded_with_warnings') : undefined;
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
		if (!silent) loading = true;
		error = null;
		try {
			const page = await service.listJobs(queryOf());
			jobs = page.items;
			total = page.total;
		} catch (problem) {
			error = messageOf(problem);
		} finally {
			loading = false;
		}
	}

	// The filter values come from the hosts, the repositories and the plans of the tenant.
	async function loadOptions(): Promise<void> {
		const [hostList, repositoryList, planList] = await Promise.allSettled([
			service.listAgents(),
			service.listRepositories(),
			service.listPlans()
		]);
		if (hostList.status === 'fulfilled') hosts = hostList.value;
		if (repositoryList.status === 'fulfilled') repositories = repositoryList.value;
		if (planList.status === 'fulfilled') plans = planList.value;
	}

	let loadedQuery = JSON.stringify(queryOf());
	onMount(() => {
		void load();
		void loadOptions();
	});

	$effect(() => {
		const key = JSON.stringify(queryOf());
		if (key === loadedQuery) return;
		loadedQuery = key;
		void load();
	});

	async function reload(): Promise<void> {
		if (loading) return;
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
	<StatusPill tone={toneOf(job)} title={fullLabel(job)}>{statusLabel(job)}</StatusPill>
{/snippet}

<Page
	title={t('jobs.title')}
	description={t('jobs.description')}
	count={loading ? undefined : total}
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
			rows={jobs}
			mode="server"
			page={tablePage}
			pageSize={PAGE_SIZE}
			{total}
			sort={tableSort}
			onPageChange={(next) => filters.update({ page: String(next) })}
			onSortChange={(next) => filters.update({ sort: next.key, order: next.direction })}
			{loading}
			{error}
			rowKey={(job) => job.id}
			rowHref={(job) => resolve('/(app)/jobs/[id]', { id: job.id })}
			actions={actionsFor}
			moreLabel={(job) => t('jobs.moreActions', { name: planLabel(job) })}
			emptyTitle={t('filter.noMatches')}
			emptyDescription={t('filter.noMatchesDescription')}
		>
			{#snippet toolbar()}
				<FilterBar
					label={t('jobs.filters')}
					value={filters.value}
					{facets}
					period
					onchange={filters.update}
					onreset={() =>
						filters.update({
							agent_id: '',
							repository_id: '',
							plan_id: '',
							kind: '',
							status: '',
							period: '',
							from: '',
							to: ''
						})}
				/>
			{/snippet}
		</DataTable>
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
