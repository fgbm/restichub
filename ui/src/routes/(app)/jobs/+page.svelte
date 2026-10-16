<script lang="ts">
	import { onMount } from 'svelte';
	import { coalesce } from '#lib/events.ts';
	import { isStreamConnected, onEvent } from '#lib/stores/events.svelte.ts';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { ApiError, service } from '#lib/api/index.ts';
	import type { Job, JobKind, JobQuery, JobStatus } from '#lib/api/index.ts';
	import { Page } from '#lib/components/layout/index.ts';
	import { Alert, Button, ConfirmDialog, FilterBar, ListView } from '#lib/components/ui/index.ts';
	import type { FilterFacet, ListMeta, RowAction } from '#lib/components/ui/index.ts';
	import { periodWindow } from '#lib/filters.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import type { MessageKey } from '#lib/i18n/index.ts';
	import { formatCount } from '#lib/format.ts';
	import { dayTime, elapsed, secondsSpan } from '#lib/list-format.ts';
	import { iconOf, isCancelled, jobState, pillOf } from '#lib/vocabulary.ts';
	import { PAGE_SIZES, lastPageIfPast, pageNumber, pageSizeOf } from '#lib/list-page.ts';
	import { queryFilters } from '#lib/stores/query-filters.svelte.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';

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

	let jobs = $state<Job[]>([]);
	let total = $state(0);
	let totalCapped = $state(false);
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
		'size',
		'sort',
		'order'
	] as const);
	const SORT_KEYS: Record<string, NonNullable<JobQuery['sort']>> = {
		started: 'started_at',
		plan: 'plan',
		host: 'host'
	};
	const sort = $derived({
		default: 'started',
		options: [
			{ value: 'started', label: t('jobs.sort.started'), order: 'desc' as const },
			{ value: 'plan', label: t('jobs.sort.plan') },
			{ value: 'host', label: t('jobs.sort.host') }
		]
	});
	// The tabs are the status groups that the server filters.
	const states = $derived({
		key: 'status',
		default: 'all',
		options: [
			{ value: 'all', label: t('jobs.tab.all') },
			{ value: 'running', label: t('jobs.tab.running') },
			{ value: 'failed', label: t('jobs.tab.failed') },
			{ value: 'succeeded', label: t('jobs.tab.succeeded') }
		]
	});
	const tablePage = $derived(pageNumber(filters.value.page));
	const pageSize = $derived(pageSizeOf(filters.value.size));

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
			sort: SORT_KEYS[value.sort || 'started'],
			order:
				value.order === 'asc' || (value.sort && value.sort !== 'started' && !value.order)
					? 'asc'
					: 'desc',
			offset: (tablePage - 1) * pageSize,
			limit: pageSize
		};
	}

	const facets = $derived<FilterFacet[]>([
		{
			key: 'agent_id',
			label: t('jobs.host'),
			options: hosts.map((host) => ({ value: host.id, label: host.hostname }))
		},
		{
			key: 'kind',
			label: t('jobDetail.kind'),
			options: KINDS.map((value) => ({ value, label: t(`jobs.kind.${value}` as MessageKey) }))
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
		}
	]);

	const FILTER_KEYS = [
		'agent_id',
		'repository_id',
		'plan_id',
		'kind',
		'status',
		'period',
		'from',
		'to'
	] as const;
	// The default period of 7 days is not a filter: an empty list then is an empty week.
	const recent = $derived(filters.value.period === '7d');
	const filtered = $derived(
		FILTER_KEYS.some((key) =>
			key === 'period' ? !recent && filters.value.period : filters.value[key]
		)
	);

	function resetFilters(): void {
		filters.update(Object.fromEntries(FILTER_KEYS.map((key) => [key, ''])));
	}

	function planLabel(job: Job): string {
		return job.plan_name ?? t(`jobs.kind.${job.kind}` as MessageKey);
	}

	function durationOf(job: Job): string {
		if (job.started_at) return elapsed(secondsSpan(job.started_at, job.finished_at));
		return cancellable(job) ? '' : '—';
	}

	function meta(job: Job): ListMeta[] {
		const at = job.started_at ?? job.run_at;
		return [
			job.agent_hostname ?? '',
			at ? { text: dayTime(at), time: at } : null,
			job.plan_name ? t(`jobs.kind.${job.kind}` as MessageKey) : null,
			job.repository_name ?? '',
			job.attempt > 1 ? t('jobs.meta.attempt', { n: job.attempt }) : null
		];
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
				icon: 'circle-stop',
				danger: true,
				onSelect: () => {
					cancelTarget = job;
					cancelOpen = true;
				}
			}
		];
	}

	function messageOf(problem: unknown): string {
		if (problem instanceof ApiError) {
			// The server pages only through the rows that it counts.
			if (problem.problem.field === 'offset') return t('filter.tooDeep');
			return tErrorCode(problem.code) ?? problem.title;
		}
		return t('table.error');
	}

	// A silent reload and a filter change can overlap; only the last request may fill the table.
	let request = 0;

	async function load(silent = false): Promise<void> {
		const mine = ++request;
		if (!silent) loading = true;
		error = null;
		try {
			const page = await service.listJobs(queryOf());
			if (mine !== request) return;
			const last = lastPageIfPast(tablePage, pageSize, page.total, page.items.length);
			if (last !== null) {
				filters.update({ page: String(last) });
				return;
			}
			jobs = page.items;
			total = page.total;
			totalCapped = page.total_capped;
		} catch (problem) {
			if (mine !== request) return;
			error = messageOf(problem);
		} finally {
			if (mine === request) loading = false;
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

	// The list opens on the jobs of the last 7 days; "All time" in the period facet shows the rest.
	if (!page.url.searchParams.has('period') && !filters.value.from && !filters.value.to) {
		filters.update({ period: '7d', page: filters.value.page });
	}

	// The key is the filter state, not the query: a period preset gives a new time on each read.
	let loadedQuery = JSON.stringify(filters.value);
	onMount(() => {
		void load();
		void loadOptions();
	});

	$effect(() => {
		const key = JSON.stringify(filters.value);
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

<Page
	title={t('jobs.title')}
	description={t('jobs.description')}
	count={loading ? undefined : totalCapped ? `${formatCount(total)}+` : total}
>
	{#snippet actions()}
		<Button icon="refresh" onclick={() => load()}>{t('common.refresh')}</Button>
	{/snippet}

	{#if totalCapped && !loading}
		<div class="capped">
			<Alert tone="info" title={t('filter.capped', { count: formatCount(total) })} />
		</div>
	{/if}

	<ListView
		label={t('jobs.title')}
		items={jobs}
		itemKey={(job) => job.id}
		icon={(job) => iconOf(jobState(job))}
		pill={(job) => pillOf(jobState(job))}
		title={planLabel}
		href={(job) => resolve('/(app)/jobs/[id]', { id: job.id })}
		{meta}
		aside={durationOf}
		actions={actionsFor}
		moreLabel={(job) => t('jobs.moreActions', { name: planLabel(job) })}
		mode="server"
		page={tablePage}
		{pageSize}
		pageSizes={PAGE_SIZES}
		{total}
		{totalCapped}
		onPageChange={(next) => filters.update({ page: String(next) })}
		onPageSizeChange={(next) => filters.update({ size: String(next) })}
		{loading}
		{error}
		{filtered}
		onReset={resetFilters}
		emptyIcon="job"
		emptyTitle={t('jobs.emptyTitle')}
		emptyDescription={t(recent ? 'jobs.emptyRecent' : 'jobs.emptyDescription')}
	>
		{#snippet emptyActions()}
			<a class="empty-link" href={resolve('/(app)/plans/new')}>{t('jobs.emptyAction')}</a>
		{/snippet}
		{#snippet toolbar()}
			<FilterBar
				label={t('jobs.filters')}
				value={filters.value}
				{states}
				{facets}
				period
				{sort}
				onchange={filters.update}
				onreset={resetFilters}
			/>
		{/snippet}
	</ListView>

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
	.empty-link {
		color: var(--rh-color-link);
	}

	.capped {
		margin-bottom: var(--rh-space-4);
	}
</style>
