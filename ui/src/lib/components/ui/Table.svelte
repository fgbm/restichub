<script lang="ts" generics="T">
	import type { Snippet } from 'svelte';
	import { t } from '#lib/i18n/index.ts';
	import EmptyState from './EmptyState.svelte';
	import Skeleton from './Skeleton.svelte';
	import type { TableColumn } from './types';

	let {
		columns,
		rows = [],
		rowKey,
		loading = false,
		caption,
		empty,
		rowActions
	}: {
		columns: TableColumn<T>[];
		rows?: T[];
		rowKey: (row: T) => string;
		loading?: boolean;
		caption?: string;
		empty?: Snippet;
		rowActions?: Snippet<[T]>;
	} = $props();

	const columnCount = $derived(columns.length + (rowActions ? 1 : 0));

	function cellOf(row: T, column: TableColumn<T>): string {
		if (column.cell) return column.cell(row);
		return String((row as Record<string, unknown>)[column.key] ?? '');
	}
</script>

<div class="table-wrap">
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
					>
						{column.header}
					</th>
				{/each}
				{#if rowActions}
					<th scope="col" class="table__cell--actions">{t('common.actions')}</th>
				{/if}
			</tr>
		</thead>
		<tbody>
			{#if loading}
				{#each Array(3) as _, index (index)}
					<tr>
						{#each columns as column (column.key)}
							<td><Skeleton /></td>
						{/each}
						{#if rowActions}
							<td><Skeleton /></td>
						{/if}
					</tr>
				{/each}
			{:else if rows.length === 0}
				<tr>
					<td colspan={columnCount}>
						{#if empty}
							{@render empty()}
						{:else}
							<EmptyState
								title={t('table.empty.title')}
								description={t('table.empty.description')}
							/>
						{/if}
					</td>
				</tr>
			{:else}
				{#each rows as row (rowKey(row))}
					<tr>
						{#each columns as column (column.key)}
							<td
								class:table__cell--right={column.align === 'right'}
								class:table__cell--center={column.align === 'center'}
								class:table__cell--mono={column.mono}
							>
								{cellOf(row, column)}
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

<style>
	.table-wrap {
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-canvas);
		overflow: hidden;
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
		padding: var(--rh-space-2) var(--rh-space-3);
		border-bottom: var(--rh-border-width) solid var(--rh-color-border);
		background-color: var(--rh-color-canvas-subtle);
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-xs);
		font-weight: var(--rh-font-weight-semibold);
		text-align: left;
		white-space: nowrap;
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

	.table__cell--right {
		text-align: right;
		font-variant-numeric: tabular-nums;
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
</style>
