<script lang="ts">
	import { formatCount } from '#lib/format.ts';
	import { t } from '#lib/i18n/index.ts';
	import Button from './Button.svelte';
	import Select from './Select.svelte';

	let {
		page,
		pageCount,
		pageSize,
		pageSizes,
		total,
		totalCapped = false,
		sizes = true,
		flush = false,
		onPage,
		onPageSize
	}: {
		page: number;
		pageCount: number;
		pageSize: number;
		pageSizes: number[];
		total: number;
		/** The server counted the rows only up to its limit: the total shows with a "+". */
		totalCapped?: boolean;
		/** Offer the choice of the page size. */
		sizes?: boolean;
		flush?: boolean;
		onPage: (page: number) => void;
		onPageSize: (pageSize: number) => void;
	} = $props();

	const rangeFrom = $derived(total === 0 ? 0 : (page - 1) * pageSize + 1);
	const rangeTo = $derived(Math.min(page * pageSize, total));
	const sizeChoices = $derived([...new Set([...pageSizes, pageSize])].sort((a, b) => a - b));
	const showSizes = $derived(sizes && total > (sizeChoices[0] ?? pageSize));
	const totalLabel = $derived(totalCapped ? `${formatCount(total)}+` : formatCount(total));

	function setPageSize(value: string): void {
		const next = Number(value);
		if (!Number.isFinite(next) || next === pageSize) return;
		onPageSize(next);
	}

	/** The first, the last and the neighbours of the current page; a gap stands for the rest. */
	const pageItems = $derived.by((): (number | null)[] => {
		if (pageCount <= 7) return Array.from({ length: pageCount }, (_, index) => index + 1);
		const from = Math.max(2, Math.min(page - 1, pageCount - 4));
		const to = Math.min(pageCount - 1, Math.max(page + 1, 5));
		const middle = Array.from({ length: to - from + 1 }, (_, index) => from + index);
		return [
			1,
			...(from > 2 ? [null] : []),
			...middle,
			...(to < pageCount - 1 ? [null] : []),
			pageCount
		];
	});
</script>

<div class="footer" class:footer--flush={flush}>
	<p class="footer__summary">
		{t('pagination.summary', {
			from: formatCount(rangeFrom),
			to: formatCount(rangeTo),
			total: totalLabel
		})}
	</p>
	<div class="footer__controls">
		{#if showSizes}
			<div class="footer__size">
				<span aria-hidden="true">{t('pagination.pageSize')}</span>
				<Select
					value={String(pageSize)}
					options={sizeChoices.map((size) => ({ value: String(size), label: String(size) }))}
					aria-label={t('pagination.pageSize')}
					onchange={(event) => setPageSize(event.currentTarget.value)}
				/>
			</div>
		{/if}
		{#if pageCount > 1}
			<nav class="footer__pager" aria-label={t('pagination.label')}>
				<Button
					size="sm"
					disabled={page <= 1}
					onclick={() => onPage(page - 1)}
					icon="chevron-left"
					aria-label={t('pagination.previous')}
				/>
				{#each pageItems as item, index (item ?? `gap-${index}`)}
					{#if item === null}
						<span class="footer__gap" aria-hidden="true">…</span>
					{:else}
						<Button
							size="sm"
							variant={item === page ? 'primary' : 'ghost'}
							aria-current={item === page ? 'page' : undefined}
							aria-label={t('pagination.page', { page: item })}
							onclick={() => onPage(item)}
						>
							{formatCount(item)}
						</Button>
					{/if}
				{/each}
				<Button
					size="sm"
					disabled={page >= pageCount}
					onclick={() => onPage(page + 1)}
					icon="chevron-right"
					aria-label={t('pagination.next')}
				/>
			</nav>
		{/if}
	</div>
</div>

<style>
	.footer {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		justify-content: space-between;
		gap: var(--rh-space-2);
		padding: var(--rh-space-2) var(--rh-space-3);
		border-top: var(--rh-border-width) solid var(--rh-color-border);
	}

	.footer--flush {
		padding: var(--rh-space-2) var(--rh-space-4);
	}

	.footer__summary {
		margin: 0;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.footer__controls {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: var(--rh-space-4);
	}

	.footer__size {
		display: flex;
		align-items: center;
		gap: var(--rh-space-2);
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
		white-space: nowrap;
	}

	.footer__size :global(.rh-select) {
		width: auto;
		height: 28px;
		padding-right: var(--rh-space-6);
		font-size: var(--rh-font-size-sm);
	}

	.footer__pager {
		display: flex;
		align-items: center;
		gap: var(--rh-space-1);
	}

	.footer__pager :global(.btn:not(.btn--icon)) {
		min-width: 28px;
		justify-content: center;
		font-variant-numeric: tabular-nums;
	}

	.footer__gap {
		min-width: 20px;
		color: var(--rh-color-text-muted);
		text-align: center;
	}
</style>
