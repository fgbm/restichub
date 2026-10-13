<script lang="ts" generics="T">
	import type { Snippet } from 'svelte';
	import { t } from '#lib/i18n/index.ts';
	import Badge from './Badge.svelte';
	import Button from './Button.svelte';
	import EmptyState from './EmptyState.svelte';
	import TextField from './TextField.svelte';
	import type { Column } from './types';

	let {
		columns,
		rows = [],
		loading = false,
		error = null,
		rowKey,
		filterable = true,
		paginated = true,
		pageSize = 10,
		emptyTitle,
		emptyDescription,
		onRowClick,
		rowActions
	}: {
		columns: Column<T>[];
		rows?: T[];
		loading?: boolean;
		error?: string | null;
		rowKey: (row: T) => string;
		filterable?: boolean;
		/** Client-side pagination. Set to `false` for cursor pagination. */
		paginated?: boolean;
		pageSize?: number;
		emptyTitle?: string;
		emptyDescription?: string;
		onRowClick?: (row: T) => void;
		rowActions?: Snippet<[T]>;
	} = $props();

	const columnCount = $derived(columns.length + (rowActions ? 1 : 0));

	let sortKey = $state<string | null>(null);
	let sortDirection = $state<'asc' | 'desc'>('asc');
	let filter = $state('');
	let page = $state(1);

	function valueOf(row: T, column: Column<T>): string {
		if (column.value) return column.value(row);
		return String((row as Record<string, unknown>)[column.key] ?? '');
	}

	function cellOf(row: T, column: Column<T>): string {
		if (column.cell) return column.cell(row);
		return valueOf(row, column);
	}

	const filtered = $derived.by(() => {
		const needle = filter.trim().toLowerCase();
		if (!needle) return rows;
		return rows.filter((row) =>
			columns.some((column) => valueOf(row, column).toLowerCase().includes(needle))
		);
	});

	const sorted = $derived.by(() => {
		if (!sortKey) return filtered;
		const column = columns.find((item) => item.key === sortKey);
		if (!column) return filtered;
		const factor = sortDirection === 'asc' ? 1 : -1;
		return [...filtered].sort(
			(a, b) => factor * valueOf(a, column).localeCompare(valueOf(b, column))
		);
	});

	const pageCount = $derived(Math.max(1, Math.ceil(sorted.length / pageSize)));
	const currentPage = $derived(Math.min(page, pageCount));
	const paged = $derived(
		paginated ? sorted.slice((currentPage - 1) * pageSize, currentPage * pageSize) : sorted
	);
	const rangeFrom = $derived(sorted.length === 0 ? 0 : (currentPage - 1) * pageSize + 1);
	const rangeTo = $derived(Math.min(currentPage * pageSize, sorted.length));

	function toggleSort(column: Column<T>): void {
		if (column.sortable === false) return;
		if (sortKey === column.key) {
			sortDirection = sortDirection === 'asc' ? 'desc' : 'asc';
		} else {
			sortKey = column.key;
			sortDirection = 'asc';
		}
	}

	function ariaSort(column: Column<T>): 'ascending' | 'descending' | 'none' {
		if (sortKey !== column.key) return 'none';
		return sortDirection === 'asc' ? 'ascending' : 'descending';
	}

	function onRowKeydown(event: KeyboardEvent, row: T): void {
		if (!onRowClick) return;
		if (event.key === 'Enter' || event.key === ' ') {
			event.preventDefault();
			onRowClick(row);
		}
	}
</script>

<div class="data-table">
	{#if filterable}
		<div class="data-table__toolbar">
			<div class="data-table__filter">
				<TextField
					type="search"
					bind:value={filter}
					placeholder={t('table.filterPlaceholder')}
					aria-label={t('common.filter')}
				/>
			</div>
		</div>
	{/if}

	<div class="data-table__scroll">
		<table>
			<thead>
				<tr>
					{#each columns as column (column.key)}
						<th
							scope="col"
							class="data-table__th"
							class:data-table__th--right={column.align === 'right'}
							aria-sort={column.sortable === false ? undefined : ariaSort(column)}
						>
							{#if column.sortable === false}
								{column.header}
							{:else}
								<button type="button" class="data-table__sort" onclick={() => toggleSort(column)}>
									{column.header}
									<span class="data-table__sort-icon" aria-hidden="true">
										{sortKey === column.key ? (sortDirection === 'asc' ? '▲' : '▼') : ''}
									</span>
								</button>
							{/if}
						</th>
					{/each}
					{#if rowActions}
						<th scope="col" class="data-table__th data-table__th--actions">{t('common.actions')}</th
						>
					{/if}
				</tr>
			</thead>
			<tbody>
				{#if loading}
					{#each Array(pageSize) as _, index (index)}
						<tr class="data-table__skeleton-row" data-testid="skeleton-row">
							{#each columns as column (column.key)}
								<td><span class="data-table__skeleton" aria-hidden="true"></span></td>
							{/each}
							{#if rowActions}
								<td><span class="data-table__skeleton" aria-hidden="true"></span></td>
							{/if}
						</tr>
					{/each}
				{:else if error}
					<tr>
						<td colspan={columnCount}>
							<span class="data-table__error" role="alert">{error}</span>
						</td>
					</tr>
				{:else if paged.length === 0}
					<tr>
						<td colspan={columnCount}>
							<EmptyState
								title={emptyTitle ?? t('table.empty.title')}
								description={emptyDescription ?? t('table.empty.description')}
							/>
						</td>
					</tr>
				{:else}
					{#each paged as row (rowKey(row))}
						<tr
							class="data-table__row"
							class:data-table__row--clickable={Boolean(onRowClick)}
							tabindex={onRowClick ? 0 : undefined}
							onclick={onRowClick ? () => onRowClick?.(row) : undefined}
							onkeydown={onRowClick ? (event) => onRowKeydown(event, row) : undefined}
						>
							{#each columns as column (column.key)}
								{@const tone = column.badge?.(row) ?? null}
								<td
									class="data-table__td"
									class:data-table__td--right={column.align === 'right'}
									class:data-table__td--mono={column.mono}
								>
									{#if tone}
										<Badge {tone}>{cellOf(row, column)}</Badge>
									{:else}
										{cellOf(row, column)}
									{/if}
								</td>
							{/each}
							{#if rowActions}
								<td class="data-table__td data-table__td--actions">{@render rowActions(row)}</td>
							{/if}
						</tr>
					{/each}
				{/if}
			</tbody>
		</table>
	</div>

	{#if paginated && !loading && !error && sorted.length > 0}
		<div class="data-table__footer">
			<p class="data-table__summary">
				{t('pagination.summary', { from: rangeFrom, to: rangeTo, total: sorted.length })}
			</p>
			<div class="data-table__pager">
				<Button
					size="sm"
					disabled={currentPage <= 1}
					onclick={() => (page = currentPage - 1)}
					aria-label={t('pagination.previous')}
				>
					{t('pagination.previous')}
				</Button>
				<span class="data-table__page"
					>{t('pagination.pageOf', {
						page: currentPage,
						pages: pageCount
					})}</span
				>
				<Button
					size="sm"
					disabled={currentPage >= pageCount}
					onclick={() => (page = currentPage + 1)}
					aria-label={t('pagination.next')}
				>
					{t('pagination.next')}
				</Button>
			</div>
		</div>
	{/if}
</div>

<style>
	.data-table {
		display: flex;
		flex-direction: column;
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-surface);
		overflow: hidden;
	}

	.data-table__toolbar {
		padding: var(--rh-space-2) var(--rh-space-3);
		border-bottom: var(--rh-border-width) solid var(--rh-color-border);
	}

	.data-table__filter {
		max-width: 20rem;
	}

	.data-table__scroll {
		overflow: auto;
		max-height: 32rem;
	}

	table {
		width: 100%;
		border-collapse: collapse;
		font-size: var(--rh-font-size-base);
	}

	.data-table__th {
		position: sticky;
		top: 0;
		z-index: var(--rh-z-sticky);
		padding: 0;
		background-color: var(--rh-color-surface-muted);
		border-bottom: var(--rh-border-width) solid var(--rh-color-border-strong);
		text-align: left;
		white-space: nowrap;
	}

	.data-table__th--right {
		text-align: right;
	}

	.data-table__th--actions,
	.data-table__td--actions {
		text-align: right;
		white-space: nowrap;
	}

	.data-table__sort {
		display: inline-flex;
		align-items: center;
		gap: var(--rh-space-1);
		width: 100%;
		padding: var(--rh-space-2) var(--rh-space-3);
		border: 0;
		background: transparent;
		color: var(--rh-color-text-muted);
		font: inherit;
		font-weight: var(--rh-font-weight-medium);
		text-align: inherit;
		cursor: pointer;
	}

	.data-table__sort:hover {
		color: var(--rh-color-text);
	}

	.data-table__sort-icon {
		font-size: var(--rh-font-size-xs);
	}

	.data-table__td {
		padding: var(--rh-space-2) var(--rh-space-3);
		border-bottom: var(--rh-border-width) solid var(--rh-color-border);
		color: var(--rh-color-text);
		vertical-align: top;
	}

	.data-table__td--right {
		text-align: right;
	}

	.data-table__td--mono {
		font-family: var(--rh-font-mono);
		font-size: var(--rh-font-size-sm);
	}

	.data-table__row:hover {
		background-color: var(--rh-color-hover);
	}

	.data-table__row--clickable {
		cursor: pointer;
	}

	.data-table__skeleton {
		display: block;
		width: 80%;
		height: 0.85em;
		border-radius: var(--rh-radius-sm);
		background: linear-gradient(
			90deg,
			var(--rh-color-skeleton),
			var(--rh-color-skeleton-shine),
			var(--rh-color-skeleton)
		);
		background-size: 200% 100%;
		animation: data-table-shimmer 1.2s ease-in-out infinite;
	}

	@keyframes data-table-shimmer {
		to {
			background-position: -200% 0;
		}
	}

	.data-table__error {
		display: block;
		padding: var(--rh-space-6) var(--rh-space-4);
		color: var(--rh-color-danger);
		text-align: center;
	}

	.data-table__footer {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		justify-content: space-between;
		gap: var(--rh-space-2);
		padding: var(--rh-space-2) var(--rh-space-3);
		border-top: var(--rh-border-width) solid var(--rh-color-border);
	}

	.data-table__summary {
		margin: 0;
		font-size: var(--rh-font-size-sm);
		color: var(--rh-color-text-muted);
	}

	.data-table__pager {
		display: flex;
		align-items: center;
		gap: var(--rh-space-2);
	}

	.data-table__page {
		font-size: var(--rh-font-size-sm);
		color: var(--rh-color-text-muted);
	}
</style>
