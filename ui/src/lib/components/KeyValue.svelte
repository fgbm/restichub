<script lang="ts">
	import type { KeyValueItem } from './types';

	let { items }: { items: KeyValueItem[] } = $props();
</script>

<dl class="key-value">
	{#each items as item (item.key)}
		<dt class="key-value__key">{item.label}</dt>
		<dd class="key-value__value" class:key-value__value--mono={item.mono}>{item.value}</dd>
	{/each}
</dl>

<style>
	.key-value {
		display: grid;
		grid-template-columns: minmax(8rem, auto) 1fr;
		gap: var(--rh-space-1) var(--rh-space-4);
		margin: 0;
	}

	.key-value__key {
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.key-value__value {
		margin: 0;
		color: var(--rh-color-text);
		font-size: var(--rh-font-size-base);
		overflow-wrap: anywhere;
	}

	.key-value__value--mono {
		font-family: var(--rh-font-mono);
		font-size: var(--rh-font-size-sm);
	}
</style>
