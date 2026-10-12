<script lang="ts">
	import { onMount } from 'svelte';
	import { ApiError, service } from '#lib/api/index.ts';
	import type { AuditActorType, AuditQuery, AuditRecord } from '#lib/api/index.ts';
	import {
		Button,
		DataTable,
		FormField,
		InlineAlert,
		JsonDiff,
		KeyValue,
		PageHeader,
		Select,
		SidePanel,
		TextField
	} from '#lib/components/index.ts';
	import type { Column, KeyValueItem, SelectOption } from '#lib/components/index.ts';
	import { getLocale, t, tErrorCode } from '#lib/i18n/index.ts';

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

	async function load(): Promise<void> {
		loading = true;
		error = null;
		try {
			const page = await fetchPage();
			items = page.items;
			nextCursor = page.next_cursor;
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
			nextCursor = page.next_cursor;
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

	const columns = $derived<Column<AuditRecord>[]>([
		{
			key: 'occurred_at',
			header: t('audit.columns.time'),
			cell: (row) => formatTime(row.occurred_at)
		},
		{
			key: 'actor',
			header: t('audit.columns.actor'),
			cell: (row) => `${row.actor_type}:${row.actor_id}`,
			mono: true
		},
		{ key: 'action', header: t('audit.columns.action'), mono: true },
		{
			key: 'object',
			header: t('audit.columns.object'),
			cell: (row) => `${row.object_type}:${row.object_id}`,
			mono: true
		},
		{ key: 'source_ip', header: t('audit.columns.sourceIp'), mono: true }
	]);

	const actorTypeOptions = $derived<SelectOption[]>([
		{ value: 'user', label: t('audit.actorType.user') },
		{ value: 'token', label: t('audit.actorType.token') },
		{ value: 'system', label: t('audit.actorType.system') }
	]);

	const detailItems = $derived<KeyValueItem[]>(
		selected
			? [
					{ key: 'id', label: 'ID', value: selected.id, mono: true },
					{ key: 'time', label: t('audit.columns.time'), value: formatTime(selected.occurred_at) },
					{
						key: 'actor',
						label: t('audit.columns.actor'),
						value: `${selected.actor_type}:${selected.actor_id}`,
						mono: true
					},
					{ key: 'action', label: t('audit.columns.action'), value: selected.action, mono: true },
					{
						key: 'object',
						label: t('audit.columns.object'),
						value: `${selected.object_type}:${selected.object_id}`,
						mono: true
					},
					{
						key: 'source_ip',
						label: t('audit.columns.sourceIp'),
						value: selected.source_ip,
						mono: true
					}
				]
			: []
	);
</script>

<PageHeader
	title={scope === 'platform' ? t('nav.platformAudit') : t('audit.title')}
	description={t('audit.description')}
/>

<form
	class="filters"
	onsubmit={(event) => {
		event.preventDefault();
		void load();
	}}
>
	<FormField label={t('audit.filters.timeFrom')} for="audit-from">
		<TextField id="audit-from" type="datetime-local" bind:value={from} />
	</FormField>
	<FormField label={t('audit.filters.timeTo')} for="audit-to">
		<TextField id="audit-to" type="datetime-local" bind:value={to} />
	</FormField>
	<FormField label={t('audit.filters.action')} for="audit-action">
		<TextField id="audit-action" bind:value={action} mono />
	</FormField>
	<FormField label={t('audit.filters.actorType')} for="audit-actor-type">
		<Select
			id="audit-actor-type"
			bind:value={actorType}
			options={actorTypeOptions}
			placeholder={t('common.all')}
		/>
	</FormField>
	<FormField label={t('audit.filters.actorId')} for="audit-actor-id">
		<TextField
			id="audit-actor-id"
			bind:value={actorId}
			placeholder={t('audit.filters.actorPlaceholder')}
			mono
		/>
	</FormField>
	<FormField label={t('audit.filters.objectType')} for="audit-object-type">
		<TextField id="audit-object-type" bind:value={objectType} mono />
	</FormField>
	<FormField label={t('audit.filters.objectId')} for="audit-object-id">
		<TextField
			id="audit-object-id"
			bind:value={objectId}
			placeholder={t('audit.filters.objectPlaceholder')}
			mono
		/>
	</FormField>
	<div class="filters__actions">
		<Button variant="primary" type="submit">{t('common.apply')}</Button>
		<Button type="button" onclick={reset}>{t('common.reset')}</Button>
	</div>
</form>

{#if error && !loading}
	<div class="alert">
		<InlineAlert tone="danger" title={error} />
	</div>
{/if}

<DataTable
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
/>

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
		<JsonDiff diff={selected.diff} />
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
		background-color: var(--rh-color-surface);
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
