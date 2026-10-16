<script lang="ts">
	import { onMount } from 'svelte';
	import { ApiError, service } from '#lib/api/index.ts';
	import type { AuditActorType, AuditQuery, AuditRecord } from '#lib/api/index.ts';
	import {
		Alert,
		Button,
		Icon,
		JsonDiff,
		KeyValue,
		PageHeader,
		Panel,
		Select,
		SidePanel,
		Table,
		TextField
	} from '#lib/components/ui/index.ts';
	import type {
		IconName,
		KeyValueItem,
		SelectOption,
		TableColumn
	} from '#lib/components/ui/index.ts';
	import { en, t, tAuditAction, tAuditActorType, tErrorCode } from '#lib/i18n/index.ts';
	import { formatTime, relativeTime } from '#lib/format.ts';

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

	const title = $derived(scope === 'platform' ? t('nav.platform.audit') : t('audit.title'));

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

	function actorLabel(row: AuditRecord): string {
		const type = tAuditActorType(row.actor_type);
		return row.actor_id ? `${type} · ${row.actor_id}` : type;
	}

	const actorIcons: Record<string, IconName> = {
		user: 'user',
		api_token: 'key',
		agent: 'host',
		system: 'settings'
	};

	function objectLabel(row: AuditRecord): string {
		return row.object_id ? `${row.object_type} ${row.object_id}` : row.object_type;
	}

	function openRecord(record: AuditRecord): void {
		selected = record;
		panelOpen = true;
	}

	async function copyRecord(): Promise<void> {
		if (!selected) return;
		try {
			await navigator.clipboard.writeText(JSON.stringify(selected, null, 2));
		} catch {
			// The clipboard needs a secure context and a permission; nothing else to do.
		}
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
			nowrap: true,
			header: t('audit.columns.time'),
			value: (row) => row.occurred_at,
			render: timeCell
		},
		{
			key: 'actor',
			header: t('audit.columns.actor'),
			value: (row) => actorLabel(row),
			sortable: false,
			render: actorCell
		},
		{
			key: 'action',
			header: t('audit.columns.action'),
			cell: (row) => tAuditAction(row.action),
			sortable: false
		},
		{
			key: 'object',
			header: t('audit.columns.object'),
			cell: (row) => objectLabel(row),
			mono: true,
			sortable: false
		},
		{
			key: 'source_ip',
			header: t('audit.columns.sourceIp'),
			cell: (row) => row.source_ip ?? t('common.unknown'),
			mono: true,
			sortable: false
		}
	]);

	const actionOptions = $derived<SelectOption[]>([
		{ value: '', label: t('audit.filters.allActions') },
		...Object.keys(en)
			.filter((key) => key.startsWith('audit.action.'))
			.map((key) => key.slice('audit.action.'.length))
			.map((value) => ({ value, label: tAuditAction(value) }))
	]);

	const actorTypeOptions = $derived<SelectOption[]>([
		{ value: '', label: t('audit.filters.allTypes') },
		{ value: 'user', label: tAuditActorType('user') },
		{ value: 'api_token', label: tAuditActorType('api_token') },
		{ value: 'agent', label: tAuditActorType('agent') },
		{ value: 'system', label: tAuditActorType('system') }
	]);

	const detailItems = $derived<KeyValueItem[]>(
		selected
			? [
					{ key: 'time', label: t('audit.columns.time'), value: formatTime(selected.occurred_at) },
					{ key: 'actor', label: t('audit.columns.actor'), value: actorLabel(selected) },
					{ key: 'action', label: t('audit.columns.action'), value: tAuditAction(selected.action) },
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
					},
					...(scope === 'platform'
						? [
								{
									key: 'tenant',
									label: t('topbar.tenant'),
									value: selected.tenant_id,
									mono: true
								}
							]
						: [])
				]
			: []
	);
</script>

<svelte:head>
	<title
		>{scope === 'platform' ? `${title} · ${t('nav.platformMode')}` : title} · {t('app.name')}</title
	>
</svelte:head>

{#snippet timeCell(row: AuditRecord)}
	<time datetime={row.occurred_at} title={formatTime(row.occurred_at)}>
		{relativeTime(row.occurred_at)}
	</time>
{/snippet}

{#snippet actorCell(row: AuditRecord)}
	<span class="actor">
		<span class="avatar" class:avatar--square={row.actor_type !== 'user'}>
			<Icon name={actorIcons[row.actor_type] ?? 'user'} size="xs" />
		</span>
		{tAuditActorType(row.actor_type)}
	</span>
	{#if row.actor_id}
		<div class="actor__id">{row.actor_type} · {row.actor_id}</div>
	{/if}
{/snippet}

<PageHeader {title} description={t('audit.description')} />

<Panel title={t('audit.filters.title')}>
	{#snippet actions()}
		<Button variant="link" onclick={reset}>{t('audit.filters.clearAll')}</Button>
	{/snippet}
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
			type="datetime-local"
			bind:value={to}
		/>
		<Select
			id="audit-action"
			label={t('audit.filters.action')}
			bind:value={action}
			options={actionOptions}
		/>
		<Select
			id="audit-actor-type"
			label={t('audit.filters.actorType')}
			bind:value={actorType}
			options={actorTypeOptions}
		/>
		<TextField
			id="audit-actor-id"
			label={t('audit.filters.actorId')}
			bind:value={actorId}
			placeholder={t('audit.filters.actorPlaceholder')}
		/>
		<TextField
			id="audit-object-type"
			label={t('audit.filters.objectType')}
			bind:value={objectType}
		/>
		<TextField
			id="audit-object-id"
			label={t('audit.filters.objectId')}
			bind:value={objectId}
			placeholder={t('audit.filters.objectPlaceholder')}
		/>
		<div class="filters__actions">
			<Button variant="primary" type="submit">{t('common.apply')}</Button>
			<Button type="button" onclick={reset}>{t('common.reset')}</Button>
		</div>
	</form>
</Panel>

{#if error && !loading}
	<div class="alert">
		<Alert tone="danger" title={error} />
	</div>
{/if}

<div class="records">
	<Table
		{columns}
		rows={items}
		{loading}
		filterable={false}
		paginated={false}
		sort={{ key: 'occurred_at', direction: 'desc' }}
		emptyIcon="audit"
		rowKey={(record) => record.id}
	>
		{#snippet rowActions(record)}
			<Button
				size="sm"
				icon="eye"
				title={t('audit.details')}
				aria-label={t('audit.detailsFor', { id: record.id })}
				onclick={() => openRecord(record)}
			/>
		{/snippet}
	</Table>
</div>

<div class="load-more">
	{#if nextCursor}
		<Button loading={loadingMore} onclick={loadMore}>{t('common.loadMore')}</Button>
	{/if}
	{#if !loading && items.length > 0}
		<p class="load-more__text">
			{nextCursor ? t('audit.loaded', { count: items.length }) : t('audit.noMore')}
		</p>
	{/if}
</div>

<SidePanel
	bind:open={panelOpen}
	title={selected ? t('audit.recordTitle', { id: selected.id }) : ''}
>
	{#if selected}
		<KeyValue items={detailItems} />
		<h3 class="detail-heading">{t('diff.title')}</h3>
		<JsonDiff diff={selected.diff ?? null} variant="lines" />
		<h3 class="detail-heading">{t('audit.oldAndNew')}</h3>
		<JsonDiff diff={selected.diff ?? null} />
	{/if}
	{#snippet footer()}
		<Button icon="copy" onclick={copyRecord}>{t('audit.copyJson')}</Button>
		<Button onclick={() => (panelOpen = false)}>{t('common.close')}</Button>
	{/snippet}
</SidePanel>

<style>
	.filters {
		display: grid;
		grid-template-columns: repeat(2, minmax(0, 1fr));
		gap: var(--rh-space-4);
	}

	@media (max-width: 720px) {
		.filters {
			grid-template-columns: minmax(0, 1fr);
		}
	}

	.filters__actions {
		display: flex;
		grid-column: 1 / -1;
		align-items: center;
		gap: var(--rh-space-2);
	}

	.alert {
		margin-top: var(--rh-space-4);
	}

	.records {
		margin-top: var(--rh-space-6);
	}

	.actor {
		display: flex;
		align-items: center;
		gap: var(--rh-space-2);
	}

	.avatar {
		display: inline-flex;
		flex: none;
		align-items: center;
		justify-content: center;
		width: 24px;
		height: 24px;
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-pill);
		background-color: var(--rh-color-neutral-bg);
		color: var(--rh-color-text-muted);
	}

	.avatar--square {
		border-radius: var(--rh-radius-sm);
	}

	.actor__id {
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.load-more {
		display: flex;
		align-items: center;
		gap: var(--rh-space-2);
		margin-top: var(--rh-space-4);
	}

	.load-more__text {
		margin: 0;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.detail-heading {
		margin: var(--rh-space-5) 0 var(--rh-space-3);
		font-size: var(--rh-font-size-lg);
		font-weight: var(--rh-font-weight-semibold);
		line-height: var(--rh-line-height-tight);
	}
</style>
