<script lang="ts">
	import { Popover as Primitive } from 'bits-ui';
	import type { VariableGroup, VariableOption } from '#lib/variables.ts';
	import VariableList from './VariableList.svelte';

	let {
		id,
		open,
		anchor,
		groups,
		activeOption = null,
		typed,
		total,
		owner,
		onpick,
		ondeclare,
		onclose,
		panel = $bindable(null)
	}: {
		id: string;
		open: boolean;
		/** The virtual element of the line of the caret; the list is a child of the body. */
		anchor: { getBoundingClientRect(): DOMRect };
		groups: VariableGroup[];
		activeOption?: VariableOption | null;
		typed: string;
		total: number;
		/** A pointer down in this element keeps the list open, because the list serves it. */
		owner: HTMLElement | null;
		onpick: (option: VariableOption) => void;
		ondeclare: (name: string) => void;
		onclose: () => void;
		panel?: HTMLDivElement | null;
	} = $props();
</script>

<!-- The list leaves the section for the body, so a dimmed section cannot make it translucent and a
     later section cannot paint over it. The floating layer of bits-ui keeps it at the caret
     (FR-PF-03). -->
{#if open}
	<Primitive.Root
		{open}
		onOpenChange={(next) => {
			if (!next) onclose();
		}}
	>
		<Primitive.Portal>
			<Primitive.Content
				customAnchor={anchor}
				align="start"
				sideOffset={2}
				collisionPadding={8}
				trapFocus={false}
				onOpenAutoFocus={(event) => event.preventDefault()}
				onCloseAutoFocus={(event) => event.preventDefault()}
				onInteractOutside={(event) => {
					const target = event.target as Node | null;
					if (target && owner?.contains(target)) event.preventDefault();
				}}
			>
				{#snippet child({ props, wrapperProps })}
					<div {...wrapperProps}>
						<div {...props} class="popover" bind:this={panel}>
							<VariableList {id} {groups} {activeOption} {typed} {total} {onpick} {ondeclare} />
						</div>
					</div>
				{/snippet}
			</Primitive.Content>
		</Primitive.Portal>
	</Primitive.Root>
{/if}

<style>
	.popover {
		position: relative;
		/* The floating wrapper copies this layer, so the list keeps its place in the body above the
		   save bar and the top bar. */
		z-index: var(--rh-z-panel);
		width: var(--bits-floating-anchor-width, 100%);
		max-width: var(--bits-floating-available-width, 100vw);
	}
</style>
