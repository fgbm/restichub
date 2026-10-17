<script lang="ts">
	import type { Snippet } from 'svelte';
	import { KeyValue, Panel } from '../ui/index.ts';
	import type { KeyValueItem } from '../ui/index.ts';

	let {
		title,
		items = [],
		actions,
		children
	}: {
		title?: string;
		/** The facts of the record, each label above its value. */
		items?: KeyValueItem[];
		actions?: Snippet;
		/** Content below the facts. */
		children?: Snippet;
	} = $props();

	const filled = $derived(items.length > 0 || Boolean(children));
</script>

<!-- A panel appears only when it has content (UI principles 3.4). -->
{#if filled}
	<Panel {title} {actions}>
		<div class="summary">
			{#if items.length > 0}
				<KeyValue {items} stacked />
			{/if}
			{@render children?.()}
		</div>
	</Panel>
{/if}

<style>
	.summary {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-3);
		min-width: 0;
	}
</style>
