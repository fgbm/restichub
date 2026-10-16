<script lang="ts">
	import Button from './Button.svelte';
	import DropdownMenu from './DropdownMenu.svelte';
	import type { MenuItem } from './types';

	let {
		items,
		moreLabel
	}: {
		items: MenuItem[];
		/** Accessible name of the … button, used when the row has three actions or more. */
		moreLabel: string;
	} = $props();

	const asButtons = $derived(items.length <= 2);
</script>

{#if items.length > 0}
	<span class="row-actions">
		{#if asButtons}
			{#each items as item (item.label)}
				<Button
					size="sm"
					variant={item.tone === 'danger' ? 'danger' : 'secondary'}
					icon={item.icon}
					disabled={item.disabled}
					title={item.hint ?? item.label}
					aria-label={item.label}
					onclick={() => item.onSelect?.()}
				/>
			{/each}
		{:else}
			<DropdownMenu icon="more" align="end" label={moreLabel} {items} />
		{/if}
	</span>
{/if}

<style>
	.row-actions {
		display: inline-flex;
		gap: var(--rh-space-1);
	}
</style>
