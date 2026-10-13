<script lang="ts">
	import { t } from '#lib/i18n/index.ts';
	import type { AuditChange, AuditDiff } from '#lib/api/index.ts';
	import Badge from './Badge.svelte';

	const REDACTED = '<redacted>';

	let {
		diff,
		variant = 'table'
	}: {
		diff: AuditDiff | null;
		/** `lines` shows a unified diff; `table` shows a box with the old and the new value of each field. */
		variant?: 'table' | 'lines';
	} = $props();

	interface Cell {
		text: string;
		redacted: boolean;
		empty: boolean;
	}

	interface Row {
		field: string;
		change: AuditChange;
	}

	function display(value: unknown): Cell {
		if (value === REDACTED) return { text: '', redacted: true, empty: false };
		if (value === null || value === undefined)
			return { text: 'null', redacted: false, empty: true };
		if (typeof value === 'string') return { text: value, redacted: false, empty: false };
		return { text: JSON.stringify(value), redacted: false, empty: false };
	}

	function isChange(value: unknown): value is AuditChange {
		return typeof value === 'object' && value !== null && ('old' in value || 'new' in value);
	}

	/**
	 * Normalize the two shapes of a diff into rows. An update holds one entry for
	 * each changed field. A create or a delete holds one `{ old, new }` pair at
	 * the top level.
	 */
	function normalize(value: AuditDiff | null): Row[] {
		if (!value) return [];
		const keys = Object.keys(value);
		const wholeRecord =
			keys.length === 2 &&
			keys.includes('old') &&
			keys.includes('new') &&
			!(isChange(value.old) && isChange(value.new));
		if (wholeRecord) {
			return [{ field: t('diff.record'), change: { old: value.old, new: value.new } }];
		}
		return Object.entries(value).map(([field, change]) => ({
			field,
			change: isChange(change) ? change : { old: null, new: change }
		}));
	}

	const rows = $derived(normalize(diff));

	interface Line {
		sign: ' ' | '-' | '+';
		text: string;
	}

	function literal(value: unknown): string {
		if (value === REDACTED) return t('diff.redacted');
		return JSON.stringify(value ?? null);
	}

	const lines = $derived.by(() => {
		const body: Line[] = [];
		for (const row of rows) {
			const label = `  ${JSON.stringify(row.field)}: `;
			const { old: before, new: after } = row.change;
			if (before === REDACTED && after === REDACTED) {
				body.push({ sign: ' ', text: label + literal(after) });
				continue;
			}
			if (before !== undefined && before !== null) {
				body.push({ sign: '-', text: label + literal(before) });
			}
			if (after !== undefined && after !== null) {
				body.push({ sign: '+', text: label + literal(after) });
			}
		}
		const last = body.length - 1;
		return [
			{ sign: ' ', text: '{' },
			...body.map((line, index) => (index < last ? { ...line, text: line.text + ',' } : line)),
			{ sign: ' ', text: '}' }
		] satisfies Line[];
	});
</script>

{#if rows.length === 0}
	<p class="diff__empty">{t('diff.empty')}</p>
{:else if variant === 'lines'}
	<div class="diff-lines">
		{#each lines as line, index (index)}
			<div
				class="diff-line"
				class:diff-line--add={line.sign === '+'}
				class:diff-line--del={line.sign === '-'}
			>
				<span class="diff-line__sign" aria-hidden="true">{line.sign}</span>{line.text}
			</div>
		{/each}
	</div>
{:else}
	<div class="diff">
		{#each rows as row (row.field)}
			{@const oldCell = display(row.change.old)}
			{@const newCell = display(row.change.new)}
			<dl class="diff__box">
				<dt>{t('diff.field')}</dt>
				<dd class="diff__mono">{row.field}</dd>
				<dt>{t('diff.old')}</dt>
				<dd class="diff__mono" class:diff__empty-value={oldCell.empty}>
					{#if oldCell.redacted}
						<Badge tone="neutral">{t('diff.redacted')}</Badge>
					{:else}
						{oldCell.text}
					{/if}
				</dd>
				<dt>{t('diff.new')}</dt>
				<dd class="diff__mono" class:diff__empty-value={newCell.empty}>
					{#if newCell.redacted}
						<Badge tone="neutral">{t('diff.redacted')}</Badge>
					{:else}
						{newCell.text}
					{/if}
				</dd>
			</dl>
		{/each}
	</div>
{/if}

<style>
	.diff {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-2);
	}

	.diff__box {
		display: grid;
		grid-template-columns: minmax(120px, auto) minmax(0, 1fr);
		margin: 0;
		padding: 0 var(--rh-space-4);
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
	}

	.diff__box dt,
	.diff__box dd {
		margin: 0;
		padding: var(--rh-space-2) 0;
		border-top: var(--rh-border-width) solid var(--rh-color-border-muted);
	}

	.diff__box dt:first-of-type,
	.diff__box dd:first-of-type {
		border-top: 0;
	}

	.diff__box dt {
		padding-right: var(--rh-space-4);
		color: var(--rh-color-text-muted);
	}

	.diff__mono {
		min-width: 0;
		font-family: var(--rh-font-mono);
		font-size: var(--rh-font-size-sm);
		overflow-wrap: anywhere;
	}

	.diff__empty-value {
		color: var(--rh-color-text-subtle);
	}

	.diff__empty {
		margin: 0;
		color: var(--rh-color-text-subtle);
	}

	.diff-lines {
		overflow: auto;
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-code-bg);
		font-family: var(--rh-font-mono);
		font-size: var(--rh-font-size-xs);
	}

	.diff-line {
		display: flex;
		gap: var(--rh-space-3);
		padding: 2px var(--rh-space-3);
		color: var(--rh-color-text-muted);
		white-space: pre;
	}

	.diff-line__sign {
		flex: none;
		width: 10px;
		color: var(--rh-color-text-subtle);
	}

	.diff-line--add {
		background-color: var(--rh-color-success-subtle);
		color: var(--rh-color-success-text);
	}

	.diff-line--del {
		background-color: var(--rh-color-danger-subtle);
		color: var(--rh-color-danger-text);
	}
</style>
