<script lang="ts">
	import { onMount } from 'svelte';
	import { ApiError, service } from '#lib/api/index.ts';
	import type { AuditActorType, AuditQuery, AuditRecord } from '#lib/api/index.ts';
	import { Page } from '#lib/components/layout/index.ts';
	import {
		Alert,
		Button,
		DataTable,
		FilterBar,
		Icon,
		JsonDiff,
		KeyValue,
		SidePanel
	} from '#lib/components/ui/index.ts';
	import type {
		FilterFacet,
		IconName,
		KeyValueItem,
		TableColumn
	} from '#lib/components/ui/index.ts';
	import { periodWindow } from '#lib/filters.ts';
	import { en, t, tAuditAction, tAuditActorType, tErrorCode } from '#lib/i18n/index.ts';
	import { formatTime } from '#lib/format.ts';
	import { queryFilters } from '#lib/stores/query-filters.svelte.ts';

	let { scope = 'tenant' }: { scope?: 'tenant' | 'platform' } = $props();

	const limit = 20;

	let items = $state<AuditRecord[]>([]);
	let nextCursor = $state<string | null>(null);
	let loading = $state(true);
	let loadingMore = $state(false);
	let error = $state<string | null>(null);
	let selected = $state<AuditRecord | null>(null);
	let panelOpen = $state(false);

	const filters = queryFilters([
		'action',
		'actor_type',
		'actor_id',
		'object_type',
		'object_id',
		'period',
		'from',
		'to'
	] as const);

	const title = $derived(scope === 'platform' ? t('nav.platform.audit') : t('audit.title'));

	function isoOrUndefined(time: number): string | undefined {
		return Number.isFinite(time) ? new Date(time).toISOString() : undefined;
	}

	function buildQuery(cursor?: string): AuditQuery {
		const value = filters.value;
		const window = periodWindow(value, Date.now());
		return {
			from: isoOrUndefined(window.start),
			to: isoOrUndefined(window.end),
			action: value.action || undefined,
			actor_type: (value.actor_type || undefined) as AuditActorType | undefined,
			actor_id: value.actor_id.trim() || undefined,
			object_type: value.object_type.trim() || undefined,
			object_id: value.object_id.trim() || undefined,
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

	// Each filter applies at once on the server; a typed identifier waits for a short pause.
	let loadedQuery: string | undefined;
	let typing: ReturnType<typeof setTimeout> | undefined;
	onMount(() => {
		loadedQuery = JSON.stringify(filters.value);
		void load();
		return () => clearTimeout(typing);
	});

	$effect(() => {
		const key = JSON.stringify(filters.value);
		if (loadedQuery === undefined || key === loadedQuery) return;
		loadedQuery = key;
		clearTimeout(typing);
		typing = setTimeout(() => void load(), 300);
	});

	const columns = $derived<TableColumn<AuditRecord>[]>([
		{
			key: 'occurred_at',
			header: t('audit.columns.time'),
			date: 'relative',
			sortable: false
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
			primary: true,
			value: (row) => tAuditAction(row.action),
			sortable: false
		},
		{
			key: 'object',
			header: t('audit.columns.object'),
			value: (row) => objectLabel(row),
			mono: true,
			truncate: 'md',
			sortable: false
		},
		{
			key: 'source_ip',
			header: t('audit.columns.sourceIp'),
			value: (row) => row.source_ip ?? t('common.unknown'),
			mono: true,
			nowrap: true,
			hideNarrow: true,
			sortable: false
		}
	]);

	const facets = $derived<FilterFacet[]>([
		{
			key: 'action',
			label: t('audit.filters.action'),
			multiple: false,
			options: Object.keys(en)
				.filter((key) => key.startsWith('audit.action.'))
				.map((key) => key.slice('audit.action.'.length))
				.map((value) => ({ value, label: tAuditAction(value) }))
		},
		{
			key: 'actor_type',
			label: t('audit.filters.actorType'),
			multiple: false,
			options: (['user', 'api_token', 'agent', 'system'] as const).map((value) => ({
				value,
				label: tAuditActorType(value)
			}))
		},
		{
			key: 'actor_id',
			label: t('audit.filters.actorId'),
			placeholder: t('audit.filters.actorPlaceholder'),
			mono: true
		},
		{ key: 'object_type', label: t('audit.filters.objectType'), mono: true },
		{
			key: 'object_id',
			label: t('audit.filters.objectId'),
			placeholder: t('audit.filters.objectPlaceholder'),
			mono: true
		}
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

<Page
	{title}
	documentTitle={scope === 'platform' ? `${title} · ${t('nav.platformMode')}` : undefined}
	description={t('audit.description')}
>
	{#if error && !loading}
		<div class="alert">
			<Alert tone="danger" title={error} />
		</div>
	{/if}

	<!-- A record has no page of its own: the row opens its details (design/style/ui-principles.md, 2.2). -->
	<DataTable
		{columns}
		rows={items}
		{loading}
		paginated={false}
		sort={{ key: 'occurred_at', direction: 'desc' }}
		emptyIcon="audit"
		rowKey={(record) => record.id}
		onRowOpen={openRecord}
	>
		{#snippet toolbar()}
			<FilterBar
				label={t('audit.filters.title')}
				value={filters.value}
				{facets}
				period
				hint={t('audit.filters.timeHint')}
				onchange={filters.update}
				onreset={filters.reset}
			/>
		{/snippet}
	</DataTable>

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
			<KeyValue items={detailItems} stacked />
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
</Page>

<style>
	.alert {
		margin-bottom: var(--rh-space-4);
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
