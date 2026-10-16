<script lang="ts">
	import type { KeyValueItem } from './types';

	let {
		items,
		boxed = false,
		stacked = false
	}: {
		items: KeyValueItem[];
		boxed?: boolean;
		/** The label above the value, for the side panel of a record page. */
		stacked?: boolean;
	} = $props();
</script>

<dl class="key-value" class:key-value--boxed={boxed} class:key-value--stacked={stacked}>
	{#each items as item (item.key)}
		<div class="key-value__row">
			<dt class="key-value__key">{item.label}</dt>
			<dd class="key-value__value" class:key-value__value--mono={item.mono}>
				{#if item.render}{@render item.render()}{:else}{item.value}{/if}
			</dd>
		</div>
	{/each}
</dl>

<style>
	.key-value {
		display: grid;
		grid-template-columns: minmax(120px, auto) minmax(0, 1fr);
		margin: 0;
	}

	.key-value__key,
	.key-value__value {
		margin: 0;
		padding: var(--rh-space-2) 0;
		border-top: var(--rh-border-width) solid var(--rh-color-border-muted);
		font-size: var(--rh-font-size-base);
	}

	.key-value__row {
		display: contents;
	}

	.key-value__row:first-child > * {
		border-top: 0;
	}

	.key-value__key {
		padding-right: var(--rh-space-4);
		color: var(--rh-color-text-muted);
	}

	.key-value__value {
		min-width: 0;
		color: var(--rh-color-text);
		overflow-wrap: anywhere;
	}

	.key-value__value--mono {
		font-family: var(--rh-font-mono);
		font-size: var(--rh-font-size-sm);
	}

	.key-value--stacked {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-3);
	}

	.key-value--stacked .key-value__row {
		display: flex;
		flex-direction: column;
		gap: 2px;
		min-width: 0;
	}

	.key-value--stacked .key-value__key,
	.key-value--stacked .key-value__value {
		padding: 0;
		border-top: 0;
	}

	.key-value--stacked .key-value__key {
		font-size: var(--rh-font-size-sm);
	}

	.key-value--boxed {
		padding: 0 var(--rh-space-4);
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
	}
</style>
