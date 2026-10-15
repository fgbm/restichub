<script lang="ts">
	import type { Snippet } from 'svelte';
	import { Popover as Primitive } from 'bits-ui';

	let {
		open = $bindable(false),
		label,
		trigger,
		children,
		align = 'start'
	}: {
		open?: boolean;
		/** Accessible name of the popover panel. */
		label: string;
		trigger: Snippet;
		children: Snippet;
		align?: 'start' | 'center' | 'end';
	} = $props();
</script>

<Primitive.Root bind:open>
	<Primitive.Trigger class="rh-popover__trigger">
		{@render trigger()}
	</Primitive.Trigger>
	<Primitive.Portal>
		<Primitive.Content class="rh-popover" {align} sideOffset={6} aria-label={label}>
			{@render children()}
		</Primitive.Content>
	</Primitive.Portal>
</Primitive.Root>

<style>
	:global(.rh-popover__trigger) {
		display: inline-flex;
		align-items: center;
		gap: var(--rh-space-2);
		padding: 0;
		border: 0;
		background: none;
		color: inherit;
		font: inherit;
		cursor: pointer;
	}

	:global(.rh-popover) {
		z-index: var(--rh-z-overlay);
		min-width: 220px;
		max-width: 360px;
		padding: var(--rh-space-3);
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-surface);
		color: var(--rh-color-text);
		font-size: var(--rh-font-size-base);
		box-shadow: var(--rh-shadow-menu);
	}
</style>
