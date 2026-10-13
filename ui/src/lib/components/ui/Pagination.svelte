<script lang="ts">
	import { t } from '#lib/i18n/index.ts';

	let {
		page = $bindable(1),
		pageCount,
		summary
	}: { page?: number; pageCount: number; summary?: string } = $props();

	const last = $derived(Math.max(1, pageCount));
	const current = $derived(Math.min(Math.max(page, 1), last));

	function go(target: number): void {
		page = Math.min(Math.max(target, 1), last);
	}
</script>

<nav class="pagination">
	{#if summary}
		<span class="pagination__summary">{summary}</span>
	{/if}
	<button
		type="button"
		class="pagination__btn"
		disabled={current <= 1}
		aria-label={t('pagination.previous')}
		onclick={() => go(current - 1)}
	>
		‹
	</button>
	<span class="pagination__page">{t('pagination.pageOf', { page: current, pages: last })}</span>
	<button
		type="button"
		class="pagination__btn"
		disabled={current >= last}
		aria-label={t('pagination.next')}
		onclick={() => go(current + 1)}
	>
		›
	</button>
</nav>

<style>
	.pagination {
		display: flex;
		align-items: center;
		gap: var(--rh-space-2);
	}

	.pagination__summary {
		margin-right: auto;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.pagination__btn {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		min-width: var(--rh-control-height);
		height: var(--rh-control-height);
		padding: 0 var(--rh-space-2);
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-control-bg);
		color: var(--rh-color-text);
		font-size: var(--rh-font-size-sm);
		font-weight: var(--rh-font-weight-semibold);
		cursor: pointer;
	}

	.pagination__btn:hover:not(:disabled) {
		background-color: var(--rh-color-control-bg-hover);
	}

	.pagination__btn:disabled {
		opacity: 0.5;
		cursor: not-allowed;
	}

	.pagination__page {
		font-size: var(--rh-font-size-sm);
		color: var(--rh-color-text-muted);
	}
</style>
