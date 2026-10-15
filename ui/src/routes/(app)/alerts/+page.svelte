<script lang="ts">
	import { onMount } from 'svelte';
	import { resolve } from '$app/paths';
	import { ApiError, service } from '#lib/api/index.ts';
	import type { Alert, AlertKind } from '#lib/api/index.ts';
	import { Button, PageHeader, Select, StatusPill, Table } from '#lib/components/ui/index.ts';
	import type { SelectOption, StatusTone, TableColumn } from '#lib/components/ui/index.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import type { MessageKey } from '#lib/i18n/index.ts';
	import { alertCondition } from '#lib/alert-text.ts';
	import { formatTime, relativeTime } from '#lib/format.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';

	type Filter = 'open' | 'resolved' | 'all';
	const PAGE_SIZE = 50;

	let alerts = $state<Alert[]>([]);
	let nextCursor = $state<string | null>(null);
	let loading = $state(true);
	let loadingMore = $state(false);
	let error = $state<string | null>(null);
	let filter = $state<Filter>('open');

	const filterOptions = $derived<SelectOption[]>(
		(['open', 'resolved', 'all'] as const).map((value) => ({
			value,
			label: t(`alerts.filter.${value}` as MessageKey)
		}))
	);

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

	function contextOf(alert: Alert): string {
		return [alert.agent_hostname, alert.repository_name]
			.filter((part) => part && part !== subjectOf(alert))
			.join(' · ');
	}

	const columns = $derived<TableColumn<Alert>[]>([
		{
			key: 'kind',
			header: t('alerts.kind'),
			value: (row) => kindLabel(row.kind),
			render: kindCell
		},
		{ key: 'subject', header: t('alerts.subject'), value: subjectOf, render: subjectCell },
		{
			key: 'message',
			header: t('alerts.message'),
			value: (row) => `${alertCondition(row)} ${row.error_text ?? ''}`.trim(),
			render: messageCell
		},
		{
			key: 'opened_at',
			nowrap: true,
			header: t('alerts.opened'),
			value: (row) => row.opened_at,
			render: openedCell
		},
		{
			key: 'status',
			header: t('alerts.state'),
			value: (row) => row.status,
			render: stateCell
		}
	]);

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

	async function load(): Promise<void> {
		loading = true;
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
			nextCursor = page.next_cursor ?? null;
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		} finally {
			loadingMore = false;
		}
	}

	onMount(load);
</script>

{#snippet kindCell(alert: Alert)}
	<span class="alert__kind">
		<StatusPill tone={alert.status === 'resolved' ? 'neutral' : TONES[alert.kind]}>
			{kindLabel(alert.kind)}
		</StatusPill>
	</span>
{/snippet}

{#snippet subjectCell(alert: Alert)}
	<span class="alert">
		{#if alert.job_id}
			<a
				class="alert__name"
				href={resolve('/(app)/jobs/[id]', { id: alert.job_id })}
				aria-label={`${t('alerts.openJob')}: ${subjectOf(alert)}`}>{subjectOf(alert)}</a
			>
		{:else}
			<span class="alert__name">{subjectOf(alert)}</span>
		{/if}
		{#if contextOf(alert)}
			<span class="alert__meta">{contextOf(alert)}</span>
		{/if}
	</span>
{/snippet}

{#snippet messageCell(alert: Alert)}
	<span class="alert">
		<span>{alertCondition(alert)}</span>
		{#if alert.error_text}
			<span class="alert__meta" title={t('alerts.errorText')}>{alert.error_text}</span>
		{/if}
	</span>
{/snippet}

{#snippet openedCell(alert: Alert)}
	<time datetime={alert.opened_at} title={formatTime(alert.opened_at)}>
		{relativeTime(alert.opened_at)}
	</time>
{/snippet}

{#snippet stateCell(alert: Alert)}
	<span class="alert">
		<StatusPill tone={alert.status === 'open' ? 'warning' : 'success'}>
			{t(`alerts.state.${alert.status}` as MessageKey)}
		</StatusPill>
		{#if alert.resolved_at}
			<span class="alert__meta" title={formatTime(alert.resolved_at)}>
				{t('alerts.resolvedAt')}: {relativeTime(alert.resolved_at)}
			</span>
		{/if}
	</span>
{/snippet}

<svelte:head>
	<title>{t('alerts.title')} · {t('app.name')}</title>
</svelte:head>

<PageHeader
	title={t('alerts.title')}
	description={t('alerts.description')}
	count={loading ? undefined : alerts.length}
>
	<Button icon="refresh" onclick={load}>{t('common.refresh')}</Button>
</PageHeader>

<Table
	{columns}
	rows={alerts}
	{loading}
	{error}
	rowKey={(alert) => alert.id}
	paginated={false}
	filterPlaceholder={t('alerts.searchPlaceholder')}
	{emptyTitle}
	emptyDescription={filter === 'open' ? t('alerts.emptyOpenBody') : undefined}
	emptyIcon="check-circle"
>
	{#snippet toolbar()}
		<div class="filter">
			<Select
				id="alerts-status"
				aria-label={t('alerts.filter')}
				options={filterOptions}
				bind:value={filter}
				onchange={load}
			/>
		</div>
	{/snippet}
</Table>
{#if nextCursor}
	<div class="more">
		<Button loading={loadingMore} onclick={loadMore}>{t('common.loadMore')}</Button>
	</div>
{/if}

<style>
	.alert {
		display: flex;
		flex-direction: column;
		align-items: flex-start;
		gap: var(--rh-space-1);
	}

	.alert__name {
		color: var(--rh-color-text);
		font-weight: var(--rh-font-weight-semibold);
		text-decoration: none;
	}

	a.alert__name:hover {
		color: var(--rh-color-link-hover);
		text-decoration: underline;
	}

	.alert__meta {
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.filter {
		min-width: 10rem;
	}

	.more {
		display: flex;
		justify-content: center;
		margin-top: var(--rh-space-4);
	}
</style>
