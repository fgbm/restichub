<script lang="ts" generics="T">
	import type { Snippet } from 'svelte';
	import { t } from '#lib/i18n/index.ts';
	import Badge from './Badge.svelte';
	import Button from './Button.svelte';
	import EmptyState from './EmptyState.svelte';
	import Icon from './Icon.svelte';
	import type { IconName } from './icons';
	import Skeleton from './Skeleton.svelte';
	import type { TableColumn } from './types';

	let {
		columns,
		rows = [],
		rowKey,
		loading = false,
		error = null,
		filterable = true,
		filterPlaceholder,
		paginated = true,
		flush = false,
		pageSize = 10,
		caption,
		emptyTitle,
		emptyDescription,
		emptyIcon,
		sort,
		onRowClick,
		rowActions,
		toolbar
	}: {
		columns: TableColumn<T>[];
		rows?: T[];
		rowKey: (row: T) => string;
		loading?: boolean;
		error?: string | null;
		filterable?: boolean;
		filterPlaceholder?: string;
		/** Client-side pagination. Set to `false` for cursor pagination. */
		paginated?: boolean;
		/** For a table inside a panel body: no outer border or radius, and cell padding as in the panel. */
		flush?: boolean;
		pageSize?: number;
		caption?: string;
		emptyTitle?: string;
		emptyDescription?: string;
		emptyIcon?: IconName;
		/** The order the rows already have, shown in the header until a person sorts. */
		sort?: { key: string; direction: 'asc' | 'desc' };
		onRowClick?: (row: T) => void;
		rowActions?: Snippet<[T]>;
		/** Controls shown in the toolbar next to the search field. */
		toolbar?: Snippet;
	} = $props();

	const columnCount = $derived(columns.length + (rowActions ? 1 : 0));

	let sortKey = $state<string | null>(null);
	let sortDirection = $state<'asc' | 'desc'>('asc');
	let userSorted = $state(false);
	const activeKey = $derived(userSorted ? sortKey : (sort?.key ?? null));
	const activeDirection = $derived(userSorted ? sortDirection : (sort?.direction ?? 'asc'));
	let filter = $state('');
	let page = $state(1);

	function valueOf(row: T, column: TableColumn<T>): string {
		if (column.value) return column.value(row);
		return String((row as Record<string, unknown>)[column.key] ?? '');
	}

	function cellOf(row: T, column: TableColumn<T>): string {
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
		if (!sortKey || !userSorted) return filtered;
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

	function toggleSort(column: TableColumn<T>): void {
		if (column.sortable === false) return;
		if (activeKey === column.key) {
			sortDirection = activeDirection === 'asc' ? 'desc' : 'asc';
		} else {
			sortDirection = 'asc';
		}
		sortKey = column.key;
		userSorted = true;
	}

	function ariaSort(column: TableColumn<T>): 'ascending' | 'descending' | 'none' {
		if (activeKey !== column.key) return 'none';
		return activeDirection === 'asc' ? 'ascending' : 'descending';
	}

	function onRowKeydown(event: KeyboardEvent, row: T): void {
		if (!onRowClick) return;
		if (event.key === 'Enter' || event.key === ' ') {
			event.preventDefault();
			onRowClick(row);
		}
	}
</script>

<div class="table-wrap" class:table-wrap--flush={flush}>
	{#if filterable || toolbar}
		<div class="table-toolbar">
			{#if filterable}
				<label class="table-search">
					<Icon name="search" />
					<input
						type="search"
						bind:value={filter}
						placeholder={filterPlaceholder ?? t('table.filterPlaceholder')}
						aria-label={t('common.filter')}
					/>
				</label>
			{/if}
			{@render toolbar?.()}
		</div>
	{/if}

	<div class="table-scroll">
		<table>
			{#if caption}
				<caption class="table__caption">{caption}</caption>
			{/if}
			<thead>
				<tr>
					{#each columns as column (column.key)}
						<th
							scope="col"
							class:table__cell--right={column.align === 'right'}
							class:table__cell--center={column.align === 'center'}
							class:table__cell--nowrap={column.nowrap}
							aria-sort={column.sortable === false ? undefined : ariaSort(column)}
						>
							{#if column.sortable === false}
								{column.header}
							{:else}
								<button type="button" class="th-btn" onclick={() => toggleSort(column)}>
									{column.header}
									<Icon name="sort" size="xs" />
								</button>
							{/if}
						</th>
					{/each}
					{#if rowActions}
						<th scope="col" class="table__cell--actions">
							<span class="table__actions-head">{t('common.actions')}</span>
						</th>
					{/if}
				</tr>
			</thead>
			<tbody>
				{#if loading}
					{#each Array(pageSize) as _, index (index)}
						<tr>
							{#each columns as column (column.key)}
								<td><Skeleton /></td>
							{/each}
							{#if rowActions}
								<td><Skeleton /></td>
							{/if}
						</tr>
					{/each}
				{:else if error}
					<tr>
						<td colspan={columnCount}>
							<span class="table__error" role="alert">{error}</span>
						</td>
					</tr>
				{:else if paged.length === 0}
					<tr>
						<td colspan={columnCount}>
							<EmptyState
								title={emptyTitle ?? t('table.empty.title')}
								description={emptyDescription ?? t('table.empty.description')}
								icon={emptyIcon}
							/>
						</td>
					</tr>
				{:else}
					{#each paged as row (rowKey(row))}
						<tr
							class:table__row--clickable={Boolean(onRowClick)}
							tabindex={onRowClick ? 0 : undefined}
							onclick={onRowClick ? () => onRowClick?.(row) : undefined}
							onkeydown={onRowClick ? (event) => onRowKeydown(event, row) : undefined}
						>
							{#each columns as column (column.key)}
								{@const tone = column.badge?.(row) ?? null}
								<td
									class:table__cell--right={column.align === 'right'}
									class:table__cell--center={column.align === 'center'}
									class:table__cell--mono={column.mono}
									class:table__cell--nowrap={column.nowrap}
								>
									{#if column.render}
										{@render column.render(row)}
									{:else if tone}
										<Badge {tone}>{cellOf(row, column)}</Badge>
									{:else}
										{cellOf(row, column)}
									{/if}
								</td>
							{/each}
							{#if rowActions}
								<td class="table__cell--actions">{@render rowActions(row)}</td>
							{/if}
						</tr>
					{/each}
				{/if}
			</tbody>
		</table>
	</div>

	{#if paginated && !loading && !error && sorted.length > 0}
		<div class="table-footer">
			<p class="table-footer__summary">
				{t('pagination.summary', { from: rangeFrom, to: rangeTo, total: sorted.length })}
			</p>
			<div class="table-footer__pager">
				<Button
					size="sm"
					disabled={currentPage <= 1}
					onclick={() => (page = currentPage - 1)}
					icon="chevron-left"
					aria-label={t('pagination.previous')}
				/>
				<span class="table-footer__page">
					{t('pagination.pageOf', { page: currentPage, pages: pageCount })}
				</span>
				<Button
					size="sm"
					disabled={currentPage >= pageCount}
					onclick={() => (page = currentPage + 1)}
					icon="chevron-right"
					aria-label={t('pagination.next')}
				/>
			</div>
		</div>
	{/if}
</div>

<style>
	.table-wrap {
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-canvas);
		overflow: hidden;
	}

	.table-wrap--flush {
		border: 0;
		border-radius: 0;
	}

	.table-wrap--flush .table-toolbar {
		padding: var(--rh-space-3) var(--rh-space-4);
	}

	.table-wrap--flush :global(th:first-child:not(:has(.th-btn))),
	.table-wrap--flush :global(th:first-child .th-btn),
	.table-wrap--flush :global(td:first-child) {
		padding-left: var(--rh-space-4);
	}

	.table-wrap--flush :global(th:last-child:not(:has(.th-btn))),
	.table-wrap--flush :global(th:last-child .th-btn),
	.table-wrap--flush :global(td:last-child) {
		padding-right: var(--rh-space-4);
	}

	.table-wrap--flush .table-footer {
		padding: var(--rh-space-2) var(--rh-space-4);
	}

	.table-toolbar {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: var(--rh-space-3);
		padding: var(--rh-space-3);
		border-bottom: var(--rh-border-width) solid var(--rh-color-border);
		background-color: var(--rh-color-canvas);
	}

	.table-search {
		display: inline-flex;
		flex: 0 1 320px;
		align-items: center;
		gap: var(--rh-space-2);
		height: var(--rh-control-height);
		padding: 0 var(--rh-space-3);
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-input-bg);
		color: var(--rh-color-text-muted);
	}

	.table-search:focus-within {
		border-color: var(--rh-color-focus);
		box-shadow: 0 0 0 3px var(--rh-color-focus-ring);
	}

	.table-search input {
		width: 100%;
		border: 0;
		outline: 0;
		background: none;
		color: var(--rh-color-text);
		font-family: inherit;
		font-size: var(--rh-font-size-base);
	}

	.table-search input::placeholder {
		color: var(--rh-color-text-subtle);
	}

	.table-scroll {
		overflow-x: auto;
	}

	table {
		width: 100%;
		border-collapse: collapse;
		font-size: var(--rh-font-size-base);
	}

	.table__caption {
		padding: var(--rh-space-2) var(--rh-space-3);
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
		text-align: left;
	}

	th {
		padding: 0;
		border-bottom: var(--rh-border-width) solid var(--rh-color-border);
		background-color: var(--rh-color-canvas-subtle);
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-xs);
		font-weight: var(--rh-font-weight-semibold);
		text-align: left;
		white-space: nowrap;
	}

	th:not(:has(.th-btn)) {
		padding: var(--rh-space-2) var(--rh-space-3);
	}

	.th-btn {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		width: 100%;
		padding: var(--rh-space-2) var(--rh-space-3);
		border: 0;
		background: transparent;
		color: inherit;
		font: inherit;
		font-weight: var(--rh-font-weight-semibold);
		text-align: inherit;
		cursor: pointer;
	}

	.th-btn :global(.icon) {
		color: var(--rh-color-text-subtle);
		opacity: 0;
	}

	.th-btn:hover :global(.icon),
	th[aria-sort='ascending'] .th-btn :global(.icon),
	th[aria-sort='descending'] .th-btn :global(.icon) {
		opacity: 1;
	}

	th[aria-sort='ascending'] .th-btn,
	th[aria-sort='descending'] .th-btn {
		color: var(--rh-color-text);
	}

	.table__actions-head {
		color: var(--rh-color-text-subtle);
	}

	td {
		padding: var(--rh-space-3);
		border-bottom: var(--rh-border-width) solid var(--rh-color-border-muted);
		vertical-align: middle;
	}

	tbody tr:last-child td {
		border-bottom: 0;
	}

	tbody tr:hover {
		background-color: var(--rh-color-hover);
	}

	.table__row--clickable {
		cursor: pointer;
	}

	.table__cell--right {
		text-align: right;
		font-variant-numeric: tabular-nums;
	}

	.table__cell--nowrap {
		white-space: nowrap;
	}

	.table__cell--center {
		text-align: center;
	}

	.table__cell--mono {
		font-family: var(--rh-font-mono);
		font-size: var(--rh-font-size-sm);
	}

	.table__cell--actions {
		width: 1%;
		text-align: right;
		white-space: nowrap;
	}

	.table__error {
		display: block;
		padding: var(--rh-space-6) var(--rh-space-4);
		color: var(--rh-color-danger);
		text-align: center;
	}

	.table-footer {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		justify-content: space-between;
		gap: var(--rh-space-2);
		padding: var(--rh-space-2) var(--rh-space-3);
		border-top: var(--rh-border-width) solid var(--rh-color-border);
	}

	.table-footer__summary {
		margin: 0;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.table-footer__pager {
		display: flex;
		align-items: center;
		gap: var(--rh-space-2);
	}

	.table-footer__page {
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}
</style>
