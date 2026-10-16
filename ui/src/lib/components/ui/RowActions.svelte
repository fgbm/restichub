<script lang="ts">
	import Button from './Button.svelte';
	import DropdownMenu from './DropdownMenu.svelte';
	import type { MenuItem } from './types';

	let {
		items,
		moreLabel,
		variant = 'secondary'
	}: {
		items: MenuItem[];
		/** Accessible name of the … button, used when the row has three actions or more. */
		moreLabel: string;
		/** `ghost`: borderless icon buttons, as in a list item. */
		variant?: 'secondary' | 'ghost';
	} = $props();

	const asButtons = $derived(items.length <= 2);
</script>

{#if items.length > 0}
	<span class="row-actions" class:row-actions--ghost={variant === 'ghost'}>
		{#if asButtons}
			{#each items as item (item.label)}
				<Button
					size="sm"
					{variant}
					icon={item.icon}
					disabled={item.disabled}
					title={item.hint ?? item.label}
					aria-label={item.label}
					class={item.tone === 'danger' ? 'row-actions__danger' : undefined}
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

	.row-actions--ghost {
		gap: 2px;
	}

	.row-actions--ghost :global(.btn),
	.row-actions--ghost :global(.rh-menu__trigger--icon) {
		width: 28px;
		height: 28px;
		color: var(--rh-color-text-muted);
	}

	.row-actions--ghost :global(.rh-menu__trigger--icon) {
		border-color: transparent;
		background-color: transparent;
	}

	.row-actions--ghost :global(.rh-menu__trigger--icon:hover),
	.row-actions--ghost :global(.rh-menu__trigger--icon[data-state='open']) {
		background-color: var(--rh-color-hover);
		color: var(--rh-color-text);
	}

	.row-actions--ghost :global(.btn:hover:not(:disabled)) {
		color: var(--rh-color-text);
	}

	.row-actions--ghost :global(.row-actions__danger:hover:not(:disabled)) {
		color: var(--rh-color-danger);
	}
</style>
