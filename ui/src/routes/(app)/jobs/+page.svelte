<script lang="ts">
	import { onMount } from 'svelte';
	import { coalesce } from '#lib/events.ts';
	import { isStreamConnected, onEvent } from '#lib/stores/events.svelte.ts';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { ApiError, service } from '#lib/api/index.ts';
	import type { Job, JobGroup, JobKind, JobListResponse, JobQuery } from '#lib/api/index.ts';
	import { Page } from '#lib/components/layout/index.ts';
	import { Alert, Button, ConfirmDialog, FilterBar, ListView } from '#lib/components/ui/index.ts';
	import type { FilterFacet, RowAction } from '#lib/components/ui/index.ts';
	import { periodWindow } from '#lib/filters.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import type { MessageKey } from '#lib/i18n/index.ts';
	import { formatCount } from '#lib/format.ts';
	import { isUnfinished, jobDuration, jobMeta, jobTitle } from '#lib/job-item.ts';
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
	let counts = $state<JobListResponse['counts'] | null>(null);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let hosts = $state<{ id: string; hostname: string }[]>([]);
	let repositories = $state<{ id: string; name: string }[]>([]);
	let plans = $state<{ id: string; name: string }[]>([]);
	// The list filters, sorts and pages on the server (FR-UI-01). The host page links here with the
	// host to show (`?agent_id=`).
	const filters = queryFilters([
		'q',
		'agent_id',
		'repository_id',
		'plan_id',
		'kind',
		'group',
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
		duration: 'duration',
		finished: 'finished_at',
		plan: 'plan',
		host: 'host'
	};
	// Newest and longest first by default; the other keys read A to Z.
	const DESCENDING = new Set(['started', 'duration', 'finished']);
	const sort = $derived({
		default: 'started',
		options: [
			{ value: 'started', label: t('jobs.sort.started'), order: 'desc' as const },
			{ value: 'duration', label: t('jobs.sort.duration'), order: 'desc' as const },
			{ value: 'finished', label: t('jobs.sort.finished'), order: 'desc' as const },
			{ value: 'plan', label: t('jobs.sort.plan') },
			{ value: 'host', label: t('jobs.sort.host') }
		]
	});
	const GROUPS: JobGroup[] = ['running', 'failed', 'succeeded_with_warnings', 'succeeded'];
	const GROUP_LABELS: Record<JobGroup, MessageKey> = {
		running: 'jobs.tab.running',
		failed: 'jobs.tab.failed',
		succeeded_with_warnings: 'jobs.tab.warnings',
		succeeded: 'jobs.tab.succeeded'
	};
	// The tabs are the status groups that the server filters and counts.
	const states = $derived({
		key: 'group',
		default: 'all',
		options: [
			{
				value: 'all',
				label: t('jobs.tab.all'),
				count: counts?.all.count,
				capped: counts?.all.capped
			},
			...GROUPS.map((group) => ({
				value: group,
				label: t(GROUP_LABELS[group]),
				count: counts?.[group].count,
				capped: counts?.[group].capped,
				danger: group === 'failed'
			}))
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
			group: GROUPS.find((group) => group === value.group),
			q: value.q || undefined,
			from: iso(window.start),
			to: iso(window.end),
			sort: SORT_KEYS[value.sort || 'started'],
			order:
				value.order === 'asc' || (!value.order && value.sort && !DESCENDING.has(value.sort))
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
		'q',
		'agent_id',
		'repository_id',
		'plan_id',
		'kind',
		'group',
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

	function actionsFor(job: Job): RowAction[] {
		if (!canWrite || !isUnfinished(job)) return [];
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
			counts = page.counts;
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
	// A typed search waits for a short pause; the other filters and the page apply at once.
	let loadedQuery = filterKey();
	let typing: ReturnType<typeof setTimeout> | undefined;
	function filterKey(): { q: string; rest: string } {
		const { q, ...rest } = filters.value;
		return { q, rest: JSON.stringify(rest) };
	}
	onMount(() => {
		void load();
		void loadOptions();
		return () => clearTimeout(typing);
	});

	$effect(() => {
		const next = filterKey();
		if (next.q === loadedQuery.q && next.rest === loadedQuery.rest) return;
		const typed = next.rest === loadedQuery.rest;
		loadedQuery = next;
		clearTimeout(typing);
		if (typed) typing = setTimeout(() => void load(), 300);
		else void load();
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
		title={jobTitle}
		href={(job) => resolve('/(app)/jobs/[id]', { id: job.id })}
		meta={(job) => jobMeta(job)}
		aside={jobDuration}
		actions={actionsFor}
		moreLabel={(job) => t('jobs.moreActions', { name: jobTitle(job) })}
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
				search={{ key: 'q', label: t('hosts.search'), placeholder: t('jobs.searchByName') }}
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
		message={t('jobs.cancelBody', { name: cancelTarget ? jobTitle(cancelTarget) : '' })}
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
