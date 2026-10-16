<script lang="ts">
	import { onMount } from 'svelte';
	import { coalesce } from '#lib/events.ts';
	import { onEvent } from '#lib/stores/events.svelte.ts';
	import { resolve } from '$app/paths';
	import { page as route } from '$app/state';
	import { ApiError, service } from '#lib/api/index.ts';
	import type { Alert, AlertKind, AlertQuery } from '#lib/api/index.ts';
	import { Page } from '#lib/components/layout/index.ts';
	import {
		Alert as Notice,
		Button,
		DataTable,
		FilterBar,
		KeyValue,
		SidePanel,
		StatusPill
	} from '#lib/components/ui/index.ts';
	import type {
		FilterFacet,
		KeyValueItem,
		StatusTone,
		TableColumn
	} from '#lib/components/ui/index.ts';
	import { queryFilters } from '#lib/stores/query-filters.svelte.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import type { MessageKey } from '#lib/i18n/index.ts';
	import { alertCondition } from '#lib/alert-text.ts';
	import { formatCount, formatTime } from '#lib/format.ts';
	import { PAGE_SIZES, lastPageIfPast, pageNumber, pageSizeOf } from '#lib/list-page.ts';

	let alerts = $state<Alert[]>([]);
	let total = $state(0);
	let totalCapped = $state(false);
	let loading = $state(true);
	let error = $state<string | null>(null);
	// The list filters and pages on the server (FR-UI-01). Without a state in the address the
	// screen opens with the open alerts; Reset shows all.
	const filters = queryFilters(['q', 'status', 'kind', 'page', 'size'] as const);
	if (!route.url.searchParams.has('status')) filters.update({ status: 'open' });
	const filter = $derived<'open' | 'resolved' | 'all'>(
		filters.value.status === 'open' || filters.value.status === 'resolved'
			? filters.value.status
			: 'all'
	);

	const tablePage = $derived(pageNumber(filters.value.page));
	const pageSize = $derived(pageSizeOf(filters.value.size));

	let detail = $state<Alert | null>(null);
	let detailOpen = $state(false);

	function openDetail(alert: Alert): void {
		detail = alert;
		detailOpen = true;
	}

	const TONES: Record<AlertKind, StatusTone> = {
		rpo_violated: 'failed',
		job_failed: 'failed',
		job_dead_letter: 'failed',
		job_warnings: 'warning',
		agent_offline: 'warning',
		stale_lock: 'warning'
	};

	function kindLabel(kind: AlertKind): string {
		return t(`alerts.kind.${kind}` as MessageKey);
	}

	function subjectOf(alert: Alert): string {
		return alert.plan_name ?? alert.repository_name ?? alert.agent_hostname ?? '—';
	}

	const columns = $derived<TableColumn<Alert>[]>([
		{
			key: 'kind',
			header: t('alerts.kind'),
			nowrap: true,
			value: (row) => kindLabel(row.kind),
			render: kindCell
		},
		{
			key: 'subject',
			header: t('alerts.subject'),
			truncate: 'sm',
			primary: true,
			value: subjectOf
		},
		{ key: 'message', header: t('alerts.message'), truncate: 'lg', value: alertCondition },
		{ key: 'opened_at', header: t('alerts.opened'), date: 'relative' },
		{
			key: 'status',
			header: t('alerts.state'),
			nowrap: true,
			value: (row) => row.status,
			render: stateCell
		}
	]);

	const facets = $derived<FilterFacet[]>([
		{
			key: 'status',
			label: t('alerts.state'),
			multiple: false,
			options: (['open', 'resolved'] as const).map((value) => ({
				value,
				label: t(`alerts.state.${value}`)
			}))
		},
		{
			key: 'kind',
			label: t('alerts.kind'),
			options: (Object.keys(TONES) as AlertKind[]).map((value) => ({
				value,
				label: kindLabel(value)
			}))
		}
	]);

	const detailItems = $derived<KeyValueItem[]>(
		detail
			? [
					{ key: 'state', label: t('alerts.state'), value: t(`alerts.state.${detail.status}`) },
					{ key: 'condition', label: t('alerts.message'), value: alertCondition(detail) },
					...(detail.error_text
						? [
								{
									key: 'error',
									label: t('alerts.errorText'),
									value: detail.error_text,
									mono: true
								}
							]
						: []),
					...(detail.plan_name
						? [{ key: 'plan', label: t('jobs.plan'), value: detail.plan_name }]
						: []),
					...(detail.agent_hostname
						? [{ key: 'host', label: t('jobs.host'), value: detail.agent_hostname }]
						: []),
					...(detail.repository_name
						? [{ key: 'repository', label: t('jobs.repository'), value: detail.repository_name }]
						: []),
					{ key: 'opened', label: t('alerts.opened'), value: formatTime(detail.opened_at) },
					...(detail.resolved_at
						? [
								{
									key: 'resolved',
									label: t('alerts.resolvedAt'),
									value: formatTime(detail.resolved_at)
								}
							]
						: [])
				]
			: []
	);

	const emptyTitle = $derived(
		filter === 'open'
			? t('alerts.emptyOpen')
			: filter === 'resolved'
				? t('alerts.emptyResolved')
				: t('alerts.emptyAll')
	);

	function messageOf(problem: unknown): string {
		if (problem instanceof ApiError) {
			// The server pages only through the alerts that it counts.
			if (problem.problem.field === 'offset') return t('filter.tooDeep');
			return tErrorCode(problem.code) ?? problem.title;
		}
		return t('table.error');
	}

	function queryOf(): AlertQuery {
		const { q, kind } = filters.value;
		return {
			status: filter,
			kind: kind || undefined,
			q: q.trim() || undefined,
			offset: (tablePage - 1) * pageSize,
			limit: pageSize
		};
	}

	// A live reload and a filter change can overlap; only the last request may fill the table.
	let request = 0;

	async function load(silent = false): Promise<void> {
		const mine = ++request;
		if (!silent) loading = true;
		error = null;
		try {
			const page = await service.listAlerts(queryOf());
			if (mine !== request) return;
			const last = lastPageIfPast(tablePage, pageSize, page.total, page.items.length);
			if (last !== null) {
				filters.update({ page: String(last) });
				return;
			}
			alerts = page.items;
			total = page.total;
			totalCapped = page.total_capped;
		} catch (problem) {
			if (mine !== request) return;
			error = messageOf(problem);
		} finally {
			if (mine === request) loading = false;
		}
	}

	// A typed search waits for a short pause; the other filters and the page apply at once.
	let loadedQuery: { q: string; rest: string } | undefined;
	let typing: ReturnType<typeof setTimeout> | undefined;
	onMount(() => {
		const { q, ...rest } = filters.value;
		loadedQuery = { q, rest: JSON.stringify(rest) };
		void load();
		return () => clearTimeout(typing);
	});

	$effect(() => {
		const { q, ...rest } = filters.value;
		const next = { q, rest: JSON.stringify(rest) };
		if (loadedQuery === undefined) return;
		if (next.q === loadedQuery.q && next.rest === loadedQuery.rest) return;
		const typed = next.rest === loadedQuery.rest;
		loadedQuery = next;
		clearTimeout(typing);
		if (typed) typing = setTimeout(() => void load(), 300);
		else void load();
	});

	const reloadSoon = coalesce(() => (loading ? undefined : load(true)), 500);
	onMount(() =>
		onEvent((event) => {
			if (event.type === 'alert.changed' || event.type === 'resync') reloadSoon();
		})
	);
</script>

{#snippet kindCell(alert: Alert)}
	<span class="alert__kind">
		<StatusPill tone={alert.status === 'resolved' ? 'neutral' : TONES[alert.kind]}>
			{kindLabel(alert.kind)}
		</StatusPill>
	</span>
{/snippet}

{#snippet stateCell(alert: Alert)}
	<StatusPill tone={alert.status === 'open' ? 'warning' : 'success'}>
		{t(`alerts.state.${alert.status}`)}
	</StatusPill>
{/snippet}

<Page
	title={t('alerts.title')}
	description={t('alerts.description')}
	count={loading ? undefined : totalCapped ? `${formatCount(total)}+` : total}
>
	{#snippet actions()}
		<Button icon="refresh" onclick={() => load()}>{t('common.refresh')}</Button>
	{/snippet}

	{#if totalCapped && !loading}
		<div class="capped">
			<Notice tone="info" title={t('filter.capped', { count: formatCount(total) })} />
		</div>
	{/if}

	<DataTable
		{columns}
		rows={alerts}
		{loading}
		{error}
		rowKey={(alert) => alert.id}
		onRowOpen={openDetail}
		mode="server"
		page={tablePage}
		{pageSize}
		pageSizes={PAGE_SIZES}
		{total}
		{totalCapped}
		onPageChange={(next) => filters.update({ page: String(next) })}
		onPageSizeChange={(next) => filters.update({ size: String(next) })}
		{emptyTitle}
		emptyDescription={filter === 'open' ? t('alerts.emptyOpenBody') : undefined}
		emptyIcon="check-circle"
	>
		{#snippet toolbar()}
			<FilterBar
				label={t('alerts.filter')}
				value={filters.value}
				search={{ key: 'q', label: t('alerts.search'), placeholder: t('alerts.searchPlaceholder') }}
				{facets}
				onchange={filters.update}
				onreset={filters.reset}
			/>
		{/snippet}
	</DataTable>

	<!-- An alert has no record page: the row opens its details (design/style/ui-principles.md, 2.2). -->
	<SidePanel
		bind:open={detailOpen}
		title={detail ? kindLabel(detail.kind) : ''}
		subtitle={detail ? subjectOf(detail) : undefined}
	>
		<KeyValue items={detailItems} stacked />
		{#snippet footer()}
			{#if detail?.job_id}
				<a class="detail__job" href={resolve('/(app)/jobs/[id]', { id: detail.job_id })}>
					{t('alerts.openJob')}
				</a>
			{/if}
		{/snippet}
	</SidePanel>
</Page>

<style>
	.capped {
		margin-bottom: var(--rh-space-4);
	}
</style>
