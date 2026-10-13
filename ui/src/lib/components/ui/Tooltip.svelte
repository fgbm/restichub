<script lang="ts">
	import type { Snippet } from 'svelte';
	import { Tooltip as Primitive } from 'bits-ui';

	let {
		content,
		ariaLabel,
		children
	}: {
		content: string;
		ariaLabel?: string;
		children: Snippet;
	} = $props();
</script>

<Primitive.Provider>
	<Primitive.Root>
		<Primitive.Trigger class="rh-tooltip__trigger" aria-label={ariaLabel}>
			{@render children()}
		</Primitive.Trigger>
		<Primitive.Portal>
			<Primitive.Content class="rh-tooltip" sideOffset={8}>
				{content}
				<Primitive.Arrow class="rh-tooltip__arrow" />
			</Primitive.Content>
		</Primitive.Portal>
	</Primitive.Root>
</Primitive.Provider>

<style>
	:global(.rh-tooltip__trigger) {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		padding: 0;
		border: 0;
		background: none;
		color: inherit;
		font: inherit;
		cursor: pointer;
	}

	:global(.rh-tooltip) {
		z-index: var(--rh-z-overlay);
		padding: var(--rh-space-1) var(--rh-space-2);
		border-radius: var(--rh-radius-sm);
		background-color: var(--rh-color-text);
		color: var(--rh-color-canvas);
		font-size: var(--rh-font-size-sm);
		box-shadow: var(--rh-shadow-menu);
	}

	:global(.rh-tooltip__arrow) {
		fill: var(--rh-color-text);
	}
</style>
