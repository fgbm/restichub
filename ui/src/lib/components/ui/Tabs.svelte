<script lang="ts">
	import type { Snippet } from 'svelte';
	import { Tabs as Primitive } from 'bits-ui';
	import type { TabItem } from './types';

	let {
		value = $bindable(''),
		items = [],
		label,
		children
	}: {
		value?: string;
		items?: TabItem[];
		label?: string;
		children?: Snippet<[string]>;
	} = $props();
</script>

<Primitive.Root bind:value>
	<Primitive.List class="rh-tabs" aria-label={label}>
		{#each items as item (item.value)}
			<Primitive.Trigger class="rh-tab" value={item.value}>
				{item.label}
				{#if item.count !== undefined}
					<span class="rh-tab__count">{item.count}</span>
				{/if}
			</Primitive.Trigger>
		{/each}
	</Primitive.List>
	{#if children}
		{#each items as item (item.value)}
			<Primitive.Content class="rh-tab-panel" value={item.value}>
				{@render children(item.value)}
			</Primitive.Content>
		{/each}
	{/if}
</Primitive.Root>

<style>
	:global(.rh-tabs) {
		display: flex;
		align-items: stretch;
		gap: var(--rh-space-1);
		border-bottom: var(--rh-border-width) solid var(--rh-color-border);
		overflow-x: auto;
	}

	:global(.rh-tab) {
		display: inline-flex;
		align-items: center;
		gap: var(--rh-space-2);
		height: 44px;
		padding: 0 var(--rh-space-3);
		margin-bottom: -1px;
		border: 0;
		border-bottom: 2px solid transparent;
		background: none;
		color: var(--rh-color-text-muted);
		font-family: inherit;
		font-size: var(--rh-font-size-base);
		white-space: nowrap;
		cursor: pointer;
	}

	:global(.rh-tab:hover) {
		color: var(--rh-color-text);
		border-bottom-color: var(--rh-color-border-strong);
	}

	:global(.rh-tab[data-state='active']) {
		color: var(--rh-color-text);
		font-weight: var(--rh-font-weight-semibold);
		border-bottom-color: var(--rh-color-accent);
	}

	:global(.rh-tab__count) {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		min-width: 18px;
		height: 18px;
		padding: 0 6px;
		border-radius: var(--rh-radius-pill);
		background-color: var(--rh-color-neutral-bg);
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-xs);
		font-weight: var(--rh-font-weight-semibold);
		font-variant-numeric: tabular-nums;
		line-height: 1;
	}

	:global(.rh-tab-panel) {
		padding-top: var(--rh-space-4);
	}
</style>
