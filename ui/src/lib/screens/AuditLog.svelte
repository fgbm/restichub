<script lang="ts">
	import { onMount } from 'svelte';
	import { ApiError, service } from '#lib/api/index.ts';
	import type { AuditActorType, AuditQuery, AuditRecord } from '#lib/api/index.ts';
	import { Page } from '#lib/components/layout/index.ts';
	import {
		Alert,
		Button,
		FilterBar,
		JsonDiff,
		KeyValue,
		ListView,
		SidePanel
	} from '#lib/components/ui/index.ts';
	import type { FilterFacet, KeyValueItem, ListMeta } from '#lib/components/ui/index.ts';
	import { clockTime, dayHeading } from '#lib/list-format.ts';
	import { AUDIT_OBJECT_ICONS } from '#lib/vocabulary.ts';
	import { periodWindow } from '#lib/filters.ts';
	import { en, t, tAuditAction, tAuditActorType, tErrorCode, tOr } from '#lib/i18n/index.ts';
	import { formatCount, formatTime } from '#lib/format.ts';
	import { PAGE_SIZES, lastPageIfPast, pageNumber, pageSizeOf } from '#lib/list-page.ts';
	import { queryFilters } from '#lib/stores/query-filters.svelte.ts';

	let { scope = 'tenant' }: { scope?: 'tenant' | 'platform' } = $props();

	let items = $state<AuditRecord[]>([]);
	let total = $state(0);
	let totalCapped = $state(false);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let selected = $state<AuditRecord | null>(null);
	let panelOpen = $state(false);

	// The log filters and pages on the server (FR-UI-01).
	const filters = queryFilters([
		'action',
		'actor_type',
		'actor_id',
		'object_type',
		'object_id',
		'period',
		'from',
		'to',
		'page',
		'size'
	] as const);
	const tablePage = $derived(pageNumber(filters.value.page));
	const pageSize = $derived(pageSizeOf(filters.value.size));

	const title = $derived(scope === 'platform' ? t('nav.platform.audit') : t('audit.title'));

	function isoOrUndefined(time: number): string | undefined {
		return Number.isFinite(time) ? new Date(time).toISOString() : undefined;
	}

	function buildQuery(): AuditQuery {
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
			offset: (tablePage - 1) * pageSize,
			limit: pageSize
		};
	}

	function fetchPage() {
		return scope === 'platform'
			? service.listPlatformAudit(buildQuery())
			: service.listAudit(buildQuery());
	}

	function messageOf(problem: unknown): string {
		if (problem instanceof ApiError) {
			// The server pages only through the records that it counts.
			if (problem.problem.field === 'offset') return t('filter.tooDeep');
			return tErrorCode(problem.code) ?? problem.title;
		}
		return t('table.error');
	}

	function actorLabel(row: AuditRecord): string {
		const type = tAuditActorType(row.actor_type);
		return row.actor_id ? `${type} · ${row.actor_id}` : type;
	}

	function objectTypeLabel(type: string): string {
		return tOr(`audit.object.${type}`, type);
	}

	// The action and the object type; the name of the object is not in the record.
	function titleOf(row: AuditRecord): string {
		return row.object_type
			? t('audit.item.title', {
					action: tAuditAction(row.action),
					object: objectTypeLabel(row.object_type)
				})
			: tAuditAction(row.action);
	}

	function meta(row: AuditRecord): ListMeta[] {
		const actor = row.actor_id
			? `${tAuditActorType(row.actor_type).toLocaleLowerCase()} ${row.actor_id}`
			: tAuditActorType(row.actor_type).toLocaleLowerCase();
		return [
			actor,
			scope === 'platform' ? { text: row.tenant_id, mono: true } : null,
			{ text: clockTime(row.occurred_at), time: row.occurred_at },
			row.source_ip ? { text: row.source_ip, mono: true } : null,
			{ text: row.action, mono: true }
		];
	}

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

	// A page change and a filter change can overlap; only the last request may fill the table.
	let request = 0;

	async function load(): Promise<void> {
		const mine = ++request;
		loading = true;
		error = null;
		try {
			const page = await fetchPage();
			if (mine !== request) return;
			const last = lastPageIfPast(tablePage, pageSize, page.total, page.items.length);
			if (last !== null) {
				filters.update({ page: String(last) });
				return;
			}
			items = page.items;
			total = page.total;
			totalCapped = page.total_capped;
		} catch (problem) {
			if (mine !== request) return;
			error = messageOf(problem);
		} finally {
			if (mine === request) loading = false;
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
		const pageOnly =
			JSON.stringify({ ...JSON.parse(loadedQuery), page: '', size: '' }) ===
			JSON.stringify({ ...filters.value, page: '', size: '' });
		loadedQuery = key;
		clearTimeout(typing);
		if (pageOnly) void load();
		else typing = setTimeout(() => void load(), 300);
	});

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

<Page
	{title}
	documentTitle={scope === 'platform' ? `${title} · ${t('nav.platformMode')}` : undefined}
	description={t('audit.description')}
>
	{#if error && !loading}
		<div class="alert">
			<Alert tone="danger" title={error} />
		</div>
	{:else if totalCapped && !loading}
		<div class="alert">
			<Alert tone="info" title={t('filter.capped', { count: formatCount(total) })} />
		</div>
	{/if}

	<!-- A record has no page of its own: the title opens its details (design/style/ui-principles.md, 2.4). -->
	<ListView
		label={title}
		{items}
		itemKey={(record) => record.id}
		icon={(record) => ({ name: AUDIT_OBJECT_ICONS[record.object_type] ?? 'audit' })}
		title={titleOf}
		onOpen={openRecord}
		{meta}
		groupBy={(record) => dayHeading(record.occurred_at)}
		{loading}
		mode="server"
		page={tablePage}
		{pageSize}
		pageSizes={PAGE_SIZES}
		{total}
		{totalCapped}
		onPageChange={(next) => filters.update({ page: String(next) })}
		onPageSizeChange={(next) => filters.update({ size: String(next) })}
		filtered={Object.entries(filters.value).some(
			([key, value]) => key !== 'page' && key !== 'size' && value
		)}
		onReset={filters.reset}
		emptyIcon="audit"
		emptyTitle={t('audit.emptyTitle')}
	>
		{#snippet toolbar()}
			<FilterBar
				label={t('audit.filters.title')}
				value={filters.value}
				total={loading ? undefined : totalCapped ? `${formatCount(total)}+` : formatCount(total)}
				{facets}
				period
				hint={t('audit.filters.timeHint')}
				onchange={filters.update}
				onreset={filters.reset}
			/>
		{/snippet}
	</ListView>

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

	.detail-heading {
		margin: var(--rh-space-5) 0 var(--rh-space-3);
		font-size: var(--rh-font-size-lg);
		font-weight: var(--rh-font-weight-semibold);
		line-height: var(--rh-line-height-tight);
	}
</style>
