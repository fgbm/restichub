<script lang="ts" generics="T">
	import type { Snippet } from 'svelte';
	import { formatLocalTime, formatTime, relativeTime } from '#lib/format.ts';
	import { t } from '#lib/i18n/index.ts';
	import Badge from './Badge.svelte';
	import Button from './Button.svelte';
	import EmptyState from './EmptyState.svelte';
	import Icon from './Icon.svelte';
	import type { IconName } from './icons';
	import RowActions from './RowActions.svelte';
	import Skeleton from './Skeleton.svelte';
	import type { MenuItem, RowAction, TableColumn, TableSort } from './types';

	let {
		columns,
		rows = [],
		rowKey,
		rowHref,
		onRowOpen,
		actions,
		moreLabel,
		mode = 'client',
		page = $bindable(1),
		pageSize = 10,
		paginated = true,
		total,
		sort = $bindable(),
		onPageChange,
		onSortChange,
		loading = false,
		error = null,
		flush = false,
		caption,
		emptyTitle,
		emptyDescription,
		emptyIcon,
		toolbar
	}: {
		columns: TableColumn<T>[];
		rows?: T[];
		rowKey: (row: T) => string;
		/** The resolved address of the record page; the row opens it through a real link. */
		rowHref?: (row: T) => string;
		/** For a table without a record page: the row opens a side panel. */
		onRowOpen?: (row: T) => void;
		/** The row actions. One or two show as icon buttons, three or more as one "…" menu. */
		actions?: (row: T) => RowAction[];
		/** The accessible name of the "…" button of a row. */
		moreLabel?: (row: T) => string;
		/** `server`: the rows are one page that the server filtered, sorted and cut. */
		mode?: 'client' | 'server';
		page?: number;
		pageSize?: number;
		/** Client mode only. Set to `false` to show all rows. */
		paginated?: boolean;
		/** Server mode: the number of rows that match the filters on all pages. */
		total?: number;
		/** The current order. In client mode, the order the rows already have until a person sorts. */
		sort?: TableSort;
		onPageChange?: (page: number) => void;
		onSortChange?: (sort: TableSort) => void;
		loading?: boolean;
		error?: string | null;
		/** For a table inside a panel body: no outer border or radius, and cell padding as in the panel. */
		flush?: boolean;
		caption?: string;
		emptyTitle?: string;
		emptyDescription?: string;
		emptyIcon?: IconName;
		/** The filter bar above the table. */
		toolbar?: Snippet;
	} = $props();

	const server = $derived(mode === 'server');
	const primaryKey = $derived((columns.find((column) => column.primary) ?? columns[0])?.key);
	const opens = $derived(Boolean(rowHref || onRowOpen));
	const columnCount = $derived(columns.length + (actions ? 1 : 0));

	let userSorted = $state(false);

	function valueOf(row: T, column: TableColumn<T>): string {
		if (column.value) return column.value(row);
		return String((row as Record<string, unknown>)[column.key] ?? '');
	}

	function cellOf(row: T, column: TableColumn<T>): string {
		if (column.cell) return column.cell(row);
		return valueOf(row, column);
	}

	const sorted = $derived.by(() => {
		if (server || !sort || !userSorted) return rows;
		const column = columns.find((item) => item.key === sort?.key);
		if (!column) return rows;
		const factor = sort.direction === 'asc' ? 1 : -1;
		return [...rows].sort((a, b) => factor * valueOf(a, column).localeCompare(valueOf(b, column)));
	});

	const totalRows = $derived(server ? (total ?? rows.length) : sorted.length);
	const showPager = $derived(server || paginated);
	const pageCount = $derived(Math.max(1, Math.ceil(totalRows / pageSize)));
	const currentPage = $derived(Math.min(Math.max(page, 1), pageCount));
	const shown = $derived(
		server || !paginated
			? sorted
			: sorted.slice((currentPage - 1) * pageSize, currentPage * pageSize)
	);
	const rangeFrom = $derived(totalRows === 0 ? 0 : (currentPage - 1) * pageSize + 1);
	const rangeTo = $derived(Math.min(currentPage * pageSize, totalRows));

	function goTo(next: number): void {
		page = next;
		onPageChange?.(next);
	}

	function toggleSort(column: TableColumn<T>): void {
		if (column.sortable === false) return;
		const direction: TableSort['direction'] =
			sort?.key === column.key && sort.direction === 'asc' ? 'desc' : 'asc';
		sort = { key: column.key, direction };
		userSorted = true;
		onSortChange?.(sort);
		if (server && page !== 1) goTo(1);
	}

	function ariaSort(column: TableColumn<T>): 'ascending' | 'descending' | 'none' {
		if (sort?.key !== column.key) return 'none';
		return sort.direction === 'asc' ? 'ascending' : 'descending';
	}

	function visibleActions(row: T): RowAction[] {
		return (actions?.(row) ?? []).filter((action) => !action.hidden);
	}

	function menuItems(list: RowAction[]): MenuItem[] {
		return list.map((action) => ({
			label: action.label,
			hint: action.hint,
			icon: action.icon,
			onSelect: action.onSelect,
			disabled: action.disabled,
			tone: action.danger ? 'danger' : 'default'
		}));
	}

	// A click on a control, a click that ends a text selection or a click in a menu outside the row
	// does not open the record.
	const CONTROLS =
		'a, button, input, select, textarea, label, summary, [role="button"], [role="menuitem"], [data-row-control]';

	function selectsText(): boolean {
		const selection = window.getSelection();
		return Boolean(selection && !selection.isCollapsed && selection.toString().trim() !== '');
	}

	function openFromRow(event: MouseEvent, row: T): void {
		if (event.defaultPrevented) return;
		if (event.type === 'auxclick' && event.button !== 1) return;
		const rowElement = event.currentTarget as HTMLElement;
		const control = (event.target as Element | null)?.closest(CONTROLS);
		if (control && rowElement.contains(control)) return;
		if (selectsText()) return;
		if (rowHref) {
			if (event.button === 1 || event.ctrlKey || event.metaKey || event.shiftKey) {
				window.open(rowHref(row), '_blank', 'noopener');
				return;
			}
			rowElement.querySelector<HTMLAnchorElement>('a.row-link')?.click();
		} else {
			onRowOpen?.(row);
		}
	}

	function truncateSize(column: TableColumn<T>): string | null {
		if (!column.truncate) return null;
		return column.truncate === true ? 'md' : column.truncate;
	}
</script>

{#snippet content(row: T, column: TableColumn<T>)}
	{@const tone = column.badge?.(row) ?? null}
	{@const size = truncateSize(column)}
	{#if column.render}
		{@render column.render(row)}
	{:else if column.date}
		{@const value = valueOf(row, column)}
		{#if value}
			<time datetime={value} title={formatTime(value)}>
				{column.date === 'relative' ? relativeTime(value) : formatLocalTime(value)}
			</time>
		{:else}
			<span class="table__none">{column.cell?.(row) || '—'}</span>
		{/if}
	{:else if tone}
		<Badge {tone}>{cellOf(row, column)}</Badge>
	{:else if size}
		{@const text = cellOf(row, column)}
		<span class="table__truncate table__truncate--{size}" title={text}>{text}</span>
	{:else}
		{cellOf(row, column)}
	{/if}
{/snippet}

<div class="table-wrap" class:table-wrap--flush={flush}>
	{@render toolbar?.()}

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
							class:table__cell--nowrap={column.nowrap || column.date}
							class:table__cell--narrow-hidden={column.hideNarrow}
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
					{#if actions}
						<th scope="col" class="table__cell--actions">
							<span class="table__actions-head">{t('common.actions')}</span>
						</th>
					{/if}
				</tr>
			</thead>
			<tbody>
				{#if loading}
					{#each Array(Math.min(pageSize, 10)) as _, index (index)}
						<tr>
							{#each Array(columnCount) as _cell, cellIndex (cellIndex)}
								<td><Skeleton /></td>
							{/each}
						</tr>
					{/each}
				{:else if error}
					<tr>
						<td colspan={columnCount}>
							<span class="table__error" role="alert">{error}</span>
						</td>
					</tr>
				{:else if shown.length === 0}
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
					{#each shown as row (rowKey(row))}
						<tr
							class:table__row--opens={opens}
							onclick={opens ? (event) => openFromRow(event, row) : undefined}
							onauxclick={rowHref ? (event) => openFromRow(event, row) : undefined}
						>
							{#each columns as column (column.key)}
								<td
									class:table__cell--right={column.align === 'right'}
									class:table__cell--center={column.align === 'center'}
									class:table__cell--mono={column.mono}
									class:table__cell--nowrap={column.nowrap || column.date}
									class:table__cell--narrow-hidden={column.hideNarrow}
								>
									{#if column.key === primaryKey && rowHref}
										<!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- rowHref returns a resolved path -->
										<a class="row-link" href={rowHref(row)}>{@render content(row, column)}</a>
									{:else if column.key === primaryKey && onRowOpen}
										<button type="button" class="row-link" onclick={() => onRowOpen?.(row)}>
											{@render content(row, column)}
										</button>
									{:else}
										{@render content(row, column)}
									{/if}
								</td>
							{/each}
							{#if actions}
								<td class="table__cell--actions" data-row-control>
									<RowActions
										items={menuItems(visibleActions(row))}
										moreLabel={moreLabel?.(row) ?? t('table.moreActions')}
									/>
								</td>
							{/if}
						</tr>
					{/each}
				{/if}
			</tbody>
		</table>
	</div>

	{#if showPager && !loading && !error && totalRows > 0}
		<div class="table-footer">
			<p class="table-footer__summary">
				{t('pagination.summary', { from: rangeFrom, to: rangeTo, total: totalRows })}
			</p>
			{#if pageCount > 1}
				<div class="table-footer__pager">
					<Button
						size="sm"
						disabled={currentPage <= 1}
						onclick={() => goTo(currentPage - 1)}
						icon="chevron-left"
						aria-label={t('pagination.previous')}
					/>
					<span class="table-footer__page">
						{t('pagination.pageOf', { page: currentPage, pages: pageCount })}
					</span>
					<Button
						size="sm"
						disabled={currentPage >= pageCount}
						onclick={() => goTo(currentPage + 1)}
						icon="chevron-right"
						aria-label={t('pagination.next')}
					/>
				</div>
			{/if}
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

	.table__row--opens {
		cursor: pointer;
	}

	/* The record link reads as the row, not as a link (UI principles 2.1). */
	.row-link {
		display: inline;
		padding: 0;
		border: 0;
		background: none;
		color: inherit;
		font: inherit;
		font-weight: var(--rh-font-weight-medium);
		text-align: inherit;
		text-decoration: none;
		cursor: pointer;
	}

	.row-link:hover {
		text-decoration: none;
	}

	.row-link:focus-visible {
		outline: 2px solid var(--rh-color-focus);
		outline-offset: 2px;
		border-radius: var(--rh-radius-sm);
	}

	.table__truncate {
		display: block;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.table__truncate--sm {
		max-width: 12rem;
	}

	.table__truncate--md {
		max-width: 20rem;
	}

	.table__truncate--lg {
		max-width: 28rem;
	}

	.table__none {
		color: var(--rh-color-text-muted);
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
		cursor: default;
	}

	@media (max-width: 720px) {
		.table__cell--narrow-hidden {
			display: none;
		}

		th.table__cell--actions,
		td.table__cell--actions {
			position: sticky;
			right: 0;
			background-color: var(--rh-color-surface);
		}
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
