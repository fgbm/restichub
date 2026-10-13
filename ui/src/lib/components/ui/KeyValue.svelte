<script lang="ts">
	import type { KeyValueItem } from './types';

	let { items, boxed = false }: { items: KeyValueItem[]; boxed?: boolean } = $props();
</script>

<dl class="key-value" class:key-value--boxed={boxed}>
	{#each items as item (item.key)}
		<dt class="key-value__key">{item.label}</dt>
		<dd class="key-value__value" class:key-value__value--mono={item.mono}>{item.value}</dd>
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

	.key-value__key:first-of-type,
	.key-value__value:first-of-type {
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

	.key-value--boxed {
		padding: 0 var(--rh-space-4);
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
	}
</style>
