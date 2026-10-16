<script lang="ts">
	import { onMount } from 'svelte';
	import { coalesce } from '#lib/events.ts';
	import { onEvent } from '#lib/stores/events.svelte.ts';
	import { resolve } from '$app/paths';
	import { page as route } from '$app/state';
	import { ApiError, service } from '#lib/api/index.ts';
	import type { Alert, AlertKind } from '#lib/api/index.ts';
	import { Page } from '#lib/components/layout/index.ts';
	import {
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
	import { facetCounts, matchesSearch, splitList } from '#lib/filters.ts';
	import { queryFilters } from '#lib/stores/query-filters.svelte.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import type { MessageKey } from '#lib/i18n/index.ts';
	import { alertCondition } from '#lib/alert-text.ts';
	import { formatTime } from '#lib/format.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';

	const PAGE_SIZE = 50;

	let alerts = $state<Alert[]>([]);
	let nextCursor = $state<string | null>(null);
	let loading = $state(true);
	let loadingMore = $state(false);
	let error = $state<string | null>(null);
	// Without a state in the address the screen opens with the open alerts; Reset shows all.
	const filters = queryFilters(['q', 'status', 'kind'] as const);
	if (!route.url.searchParams.has('status')) filters.update({ status: 'open' });
	const filter = $derived<'open' | 'resolved' | 'all'>(
		filters.value.status === 'open' || filters.value.status === 'resolved'
			? filters.value.status
			: 'all'
	);

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
			options: facetCounts(alerts, (alert) => [alert.kind]).map((option) => ({
				...option,
				label: kindLabel(option.value as AlertKind)
			}))
		}
	]);

	const shown = $derived(
		alerts.filter((alert) => {
			const { q, kind } = filters.value;
			if (kind && !splitList(kind).includes(alert.kind)) return false;
			return matchesSearch(q, [
				kindLabel(alert.kind),
				subjectOf(alert),
				alert.agent_hostname,
				alert.repository_name,
				alertCondition(alert)
			]);
		})
	);

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
			const page = await service.listAlerts({ status: filter, limit: PAGE_SIZE });
			alerts = page.items;
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
			const page = await service.listAlerts({
				status: filter,
				limit: PAGE_SIZE,
				cursor: nextCursor
			});
			alerts = [...alerts, ...page.items];
			extended = true;
			nextCursor = page.next_cursor ?? null;
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		} finally {
			loadingMore = false;
		}
	}

	let loadedFilter: string | undefined;
	onMount(() => {
		loadedFilter = filter;
		void load();
	});

	// The state filters on the server; the search and the kind narrow the loaded rows.
	$effect(() => {
		const key = filter;
		if (loadedFilter === undefined || key === loadedFilter) return;
		loadedFilter = key;
		void load();
	});

	// A page that the user extended with "load more" shows its rows until the next full load.
	let extended = false;
	const reloadSoon = coalesce(
		() => (extended || loading || loadingMore ? undefined : load(true)),
		500
	);
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
	count={loading ? undefined : alerts.length}
>
	{#snippet actions()}
		<Button icon="refresh" onclick={() => load()}>{t('common.refresh')}</Button>
	{/snippet}

	<DataTable
		{columns}
		rows={shown}
		{loading}
		{error}
		rowKey={(alert) => alert.id}
		onRowOpen={openDetail}
		paginated={false}
		{emptyTitle}
		emptyDescription={filter === 'open' ? t('alerts.emptyOpenBody') : undefined}
		emptyIcon="check-circle"
	>
		{#snippet toolbar()}
			<FilterBar
				label={t('alerts.filter')}
				value={filters.value}
				search={{ key: 'q', label: t('hosts.search'), placeholder: t('alerts.searchPlaceholder') }}
				{facets}
				onchange={filters.update}
				onreset={filters.reset}
			/>
		{/snippet}
	</DataTable>
	{#if nextCursor}
		<div class="more">
			<Button loading={loadingMore} onclick={loadMore}>{t('common.loadMore')}</Button>
		</div>
	{/if}

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
	.more {
		display: flex;
		justify-content: center;
		margin-top: var(--rh-space-4);
	}
</style>
