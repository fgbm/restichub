<script lang="ts">
	import Badge from './Badge.svelte';
	import { t } from '#lib/i18n/index.ts';
	import type { AuditDiff } from '#lib/api/index.ts';

	/** The control plane replaces a secret value with this marker. */
	const REDACTED = '<redacted>';

	let { diff }: { diff: AuditDiff | null } = $props();

	interface Cell {
		text: string;
		redacted: boolean;
		empty: boolean;
	}

	function display(value: unknown): Cell {
		if (value === REDACTED) return { text: '', redacted: true, empty: false };
		if (value === null || value === undefined)
			return { text: 'null', redacted: false, empty: true };
		if (typeof value === 'string') return { text: value, redacted: false, empty: false };
		return { text: JSON.stringify(value), redacted: false, empty: false };
	}

	const entries = $derived(diff ? Object.entries(diff) : []);
</script>

{#if entries.length === 0}
	<p class="diff__empty">{t('diff.empty')}</p>
{:else}
	<table class="diff">
		<thead>
			<tr>
				<th scope="col">{t('diff.field')}</th>
				<th scope="col">{t('diff.old')}</th>
				<th scope="col">{t('diff.new')}</th>
			</tr>
		</thead>
		<tbody>
			{#each entries as [field, change] (field)}
				{@const oldCell = display(change.old)}
				{@const newCell = display(change.new)}
				<tr>
					<th scope="row" class="diff__field">{field}</th>
					<td class:diff__empty-value={oldCell.empty}>
						{#if oldCell.redacted}
							<Badge tone="neutral">{t('diff.redacted')}</Badge>
						{:else}
							{oldCell.text}
						{/if}
					</td>
					<td class:diff__empty-value={newCell.empty}>
						{#if newCell.redacted}
							<Badge tone="neutral">{t('diff.redacted')}</Badge>
						{:else}
							{newCell.text}
						{/if}
					</td>
				</tr>
			{/each}
		</tbody>
	</table>
{/if}

<style>
	.diff {
		width: 100%;
		border-collapse: collapse;
		font-size: var(--rh-font-size-sm);
	}

	.diff th,
	.diff td {
		padding: var(--rh-space-1) var(--rh-space-2);
		border-bottom: var(--rh-border-width) solid var(--rh-color-border);
		text-align: left;
		vertical-align: top;
		overflow-wrap: anywhere;
	}

	.diff thead th {
		color: var(--rh-color-text-muted);
		font-weight: var(--rh-font-weight-medium);
	}

	.diff__field {
		font-family: var(--rh-font-mono);
		font-weight: var(--rh-font-weight-regular);
		color: var(--rh-color-text);
	}

	.diff__empty-value {
		color: var(--rh-color-text-subtle);
		font-family: var(--rh-font-mono);
	}

	.diff__empty {
		margin: 0;
		color: var(--rh-color-text-subtle);
	}
</style>
