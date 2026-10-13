<script lang="ts">
	import { onMount } from 'svelte';
	import { ApiError, service } from '#lib/api/index.ts';
	import type { AuditActorType, AuditQuery, AuditRecord } from '#lib/api/index.ts';
	import {
		Alert,
		Button,
		JsonDiff,
		KeyValue,
		PageHeader,
		Select,
		SidePanel,
		Table,
		TextField
	} from '#lib/components/ui/index.ts';
	import type { KeyValueItem, SelectOption, TableColumn } from '#lib/components/ui/index.ts';
	import { getLocale, t, tAuditAction, tAuditActorType, tErrorCode } from '#lib/i18n/index.ts';

	let { scope = 'tenant' }: { scope?: 'tenant' | 'platform' } = $props();

	const limit = 20;

	let items = $state<AuditRecord[]>([]);
	let nextCursor = $state<string | null>(null);
	let loading = $state(true);
	let loadingMore = $state(false);
	let error = $state<string | null>(null);
	let selected = $state<AuditRecord | null>(null);
	let panelOpen = $state(false);

	let from = $state('');
	let to = $state('');
	let action = $state('');
	let actorType = $state('');
	let actorId = $state('');
	let objectType = $state('');
	let objectId = $state('');

	const title = $derived(scope === 'platform' ? t('nav.platformAudit') : t('audit.title'));

	function isoOrUndefined(value: string): string | undefined {
		return value ? new Date(value).toISOString() : undefined;
	}

	function buildQuery(cursor?: string): AuditQuery {
		return {
			from: isoOrUndefined(from),
			to: isoOrUndefined(to),
			action: action || undefined,
			actor_type: (actorType || undefined) as AuditActorType | undefined,
			actor_id: actorId || undefined,
			object_type: objectType || undefined,
			object_id: objectId || undefined,
			cursor,
			limit
		};
	}

	function fetchPage(cursor?: string) {
		return scope === 'platform'
			? service.listPlatformAudit(buildQuery(cursor))
			: service.listAudit(buildQuery(cursor));
	}

	function messageOf(problem: unknown): string {
		if (problem instanceof ApiError) return tErrorCode(problem.code) ?? problem.title;
		return t('table.error');
	}

	function formatTime(value: string): string {
		return new Date(value).toLocaleString(getLocale());
	}

	function actorLabel(row: AuditRecord): string {
		const type = tAuditActorType(row.actor_type);
		return row.actor_id ? `${type}: ${row.actor_id}` : type;
	}

	function objectLabel(row: AuditRecord): string {
		return row.object_id ? `${row.object_type}: ${row.object_id}` : row.object_type;
	}

	async function load(): Promise<void> {
		loading = true;
		error = null;
		try {
			const page = await fetchPage();
			items = page.items;
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
			const page = await fetchPage(nextCursor);
			items = [...items, ...page.items];
			nextCursor = page.next_cursor ?? null;
		} catch (problem) {
			error = messageOf(problem);
		} finally {
			loadingMore = false;
		}
	}

	function reset(): void {
		from = '';
		to = '';
		action = '';
		actorType = '';
		actorId = '';
		objectType = '';
		objectId = '';
		void load();
	}

	onMount(load);

	const columns = $derived<TableColumn<AuditRecord>[]>([
		{
			key: 'occurred_at',
			header: t('audit.columns.time'),
			cell: (row) => formatTime(row.occurred_at)
		},
		{
			key: 'actor',
			header: t('audit.columns.actor'),
			cell: (row) => actorLabel(row),
			mono: true
		},
		{
			key: 'action',
			header: t('audit.columns.action'),
			cell: (row) => tAuditAction(row.action),
			mono: true
		},
		{
			key: 'object',
			header: t('audit.columns.object'),
			cell: (row) => objectLabel(row),
			mono: true
		},
		{
			key: 'source_ip',
			header: t('audit.columns.sourceIp'),
			cell: (row) => row.source_ip ?? t('common.unknown'),
			mono: true
		}
	]);

	const actorTypeOptions = $derived<SelectOption[]>([
		{ value: 'user', label: tAuditActorType('user') },
		{ value: 'api_token', label: tAuditActorType('api_token') },
		{ value: 'agent', label: tAuditActorType('agent') },
		{ value: 'system', label: tAuditActorType('system') }
	]);

	const detailItems = $derived<KeyValueItem[]>(
		selected
			? [
					{ key: 'id', label: 'ID', value: selected.id, mono: true },
					{ key: 'tenant', label: t('topbar.tenant'), value: selected.tenant_id, mono: true },
					{ key: 'time', label: t('audit.columns.time'), value: formatTime(selected.occurred_at) },
					{
						key: 'actor',
						label: t('audit.columns.actor'),
						value: actorLabel(selected),
						mono: true
					},
					{
						key: 'action',
						label: t('audit.columns.action'),
						value: tAuditAction(selected.action),
						mono: true
					},
					{
						key: 'object',
						label: t('audit.columns.object'),
						value: objectLabel(selected),
						mono: true
					},
					{
						key: 'source_ip',
						label: t('audit.columns.sourceIp'),
						value: selected.source_ip ?? t('common.unknown'),
						mono: true
					}
				]
			: []
	);
</script>

<svelte:head>
	<title>{title} · {t('app.name')}</title>
</svelte:head>

<PageHeader {title} description={t('audit.description')}>
	<Button icon="i-refresh" {loading} onclick={load}>{t('common.refresh')}</Button>
</PageHeader>

<form
	class="filters"
	onsubmit={(event) => {
		event.preventDefault();
		void load();
	}}
>
	<TextField
		id="audit-from"
		label={t('audit.filters.timeFrom')}
		help={t('audit.filters.timeHint')}
		type="datetime-local"
		bind:value={from}
	/>
	<TextField
		id="audit-to"
		label={t('audit.filters.timeTo')}
		help={t('audit.filters.timeHint')}
		type="datetime-local"
		bind:value={to}
	/>
	<TextField id="audit-action" label={t('audit.filters.action')} bind:value={action} mono />
	<Select
		id="audit-actor-type"
		label={t('audit.filters.actorType')}
		bind:value={actorType}
		options={actorTypeOptions}
		placeholder={t('common.all')}
	/>
	<TextField
		id="audit-actor-id"
		label={t('audit.filters.actorId')}
		bind:value={actorId}
		placeholder={t('audit.filters.actorPlaceholder')}
		mono
	/>
	<TextField
		id="audit-object-type"
		label={t('audit.filters.objectType')}
		bind:value={objectType}
		mono
	/>
	<TextField
		id="audit-object-id"
		label={t('audit.filters.objectId')}
		bind:value={objectId}
		placeholder={t('audit.filters.objectPlaceholder')}
		mono
	/>
	<div class="filters__actions">
		<Button variant="primary" type="submit">{t('common.apply')}</Button>
		<Button type="button" onclick={reset}>{t('common.reset')}</Button>
	</div>
</form>

{#if error && !loading}
	<div class="alert">
		<Alert tone="danger" title={error} />
	</div>
{/if}

<Table
	{columns}
	rows={items}
	{loading}
	filterable={false}
	paginated={false}
	rowKey={(record) => record.id}
	onRowClick={(record) => {
		selected = record;
		panelOpen = true;
	}}
>
	{#snippet rowActions(record)}
		<Button
			size="sm"
			icon="i-eye"
			onclick={() => {
				selected = record;
				panelOpen = true;
			}}
		>
			{t('audit.details')}
		</Button>
	{/snippet}
</Table>

<div class="load-more">
	{#if nextCursor}
		<Button loading={loadingMore} onclick={loadMore}>{t('common.loadMore')}</Button>
	{:else if !loading && items.length > 0}
		<p class="load-more__end">{t('audit.noMore')}</p>
	{/if}
</div>

<SidePanel
	bind:open={panelOpen}
	title={t('audit.detailTitle')}
	subtitle={selected ? t('audit.detailSubtitle', { id: selected.id }) : undefined}
>
	{#if selected}
		<KeyValue items={detailItems} />
		<h3 class="detail-heading">{t('diff.title')}</h3>
		<JsonDiff diff={selected.diff ?? null} />
	{/if}
	{#snippet footer()}
		<Button onclick={() => (panelOpen = false)}>{t('common.close')}</Button>
	{/snippet}
</SidePanel>

<style>
	.filters {
		display: grid;
		grid-template-columns: repeat(auto-fit, minmax(12rem, 1fr));
		gap: var(--rh-space-3);
		margin-bottom: var(--rh-space-4);
		padding: var(--rh-space-4);
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-canvas-subtle);
	}

	.filters__actions {
		display: flex;
		align-items: flex-end;
		gap: var(--rh-space-2);
	}

	.alert {
		margin-bottom: var(--rh-space-4);
	}

	.load-more {
		display: flex;
		justify-content: center;
		margin-top: var(--rh-space-4);
	}

	.load-more__end {
		margin: 0;
		color: var(--rh-color-text-subtle);
		font-size: var(--rh-font-size-sm);
	}

	.detail-heading {
		margin: var(--rh-space-5) 0 var(--rh-space-2);
		font-size: var(--rh-font-size-md);
		font-weight: var(--rh-font-weight-semibold);
	}
</style>
