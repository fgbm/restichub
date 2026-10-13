<script lang="ts">
	import type { Snippet } from 'svelte';
	import { DropdownMenu as Primitive } from 'bits-ui';
	import Icon from './Icon.svelte';
	import type { IconName } from './icons';
	import type { MenuItem } from './types';

	let {
		items = [],
		align = 'start',
		icon,
		label,
		menuClass = '',
		header,
		children
	}: {
		items?: MenuItem[];
		align?: 'start' | 'center' | 'end';
		/** Without children the trigger is a small icon button and needs `label`. */
		icon?: IconName;
		label?: string;
		/** Extra class on the floating menu, for example a wider minimum width. */
		menuClass?: string;
		/** Static content above the items, such as the account name. */
		header?: Snippet;
		children?: Snippet;
	} = $props();

	interface Block {
		group?: string;
		items: MenuItem[];
	}

	// Items that share a `group` are exclusive, so they render as radio items.
	const blocks = $derived.by(() => {
		const result: Block[] = [];
		for (const item of items) {
			const last = result[result.length - 1];
			if (item.group && last?.group === item.group) {
				last.items.push(item);
			} else {
				result.push({ group: item.group, items: [item] });
			}
		}
		return result;
	});

	function checkedValue(block: Block): string {
		return block.items.find((item) => item.checked)?.label ?? '';
	}
</script>

<Primitive.Root>
	<Primitive.Trigger
		class="rh-menu__trigger{children ? '' : ' rh-menu__trigger--icon'}"
		aria-label={label}
	>
		{#if icon}<Icon name={icon} size={children ? 'md' : 'xs'} />{/if}
		{@render children?.()}
	</Primitive.Trigger>
	<Primitive.Portal>
		<Primitive.Content class="rh-menu {menuClass}" {align} sideOffset={6}>
			{#if header}
				<div class="rh-menu__head">{@render header()}</div>
				<Primitive.Separator class="rh-menu__separator" />
			{/if}
			{#each blocks as block, blockIndex (blockIndex)}
				{#if block.group}
					{#if block.items[0].separatorBefore}
						<Primitive.Separator class="rh-menu__separator" />
					{/if}
					<Primitive.RadioGroup value={checkedValue(block)} onValueChange={() => {}}>
						{#if block.items[0].groupLabel}
							<Primitive.GroupHeading class="rh-menu__label">
								{block.items[0].groupLabel}
							</Primitive.GroupHeading>
						{/if}
						{#each block.items as item (item.label)}
							<Primitive.RadioItem
								class="rh-menu__item"
								value={item.label}
								disabled={item.disabled}
								onSelect={item.onSelect}
							>
								{#if item.icon}<Icon name={item.icon} />{/if}
								{item.label}
								{#if item.checked}
									<span class="rh-menu__check"><Icon name="check" /></span>
								{/if}
							</Primitive.RadioItem>
						{/each}
					</Primitive.RadioGroup>
				{:else}
					{#each block.items as item (item.label)}
						{#if item.separatorBefore}
							<Primitive.Separator class="rh-menu__separator" />
						{/if}
						<Primitive.Item
							class="rh-menu__item{item.tone === 'danger' ? ' rh-menu__item--danger' : ''}"
							disabled={item.disabled}
							onSelect={item.onSelect}
						>
							{#if item.icon}<Icon name={item.icon} />{/if}
							{item.label}
						</Primitive.Item>
					{/each}
				{/if}
			{/each}
		</Primitive.Content>
	</Primitive.Portal>
</Primitive.Root>

<style>
	:global(.rh-menu__trigger) {
		display: inline-flex;
		align-items: center;
		gap: var(--rh-space-2);
		height: var(--rh-control-height);
		padding: 0 var(--rh-space-2);
		border: var(--rh-border-width) solid transparent;
		border-radius: var(--rh-radius-md);
		background: transparent;
		color: var(--rh-color-text);
		font-family: inherit;
		font-size: var(--rh-font-size-base);
		cursor: pointer;
	}

	:global(.rh-menu__trigger:hover),
	:global(.rh-menu__trigger[data-state='open']) {
		background-color: var(--rh-color-hover);
	}

	:global(.rh-menu__trigger--icon) {
		justify-content: center;
		width: 28px;
		height: 28px;
		padding: 0;
		border-color: var(--rh-color-border);
		background-color: var(--rh-color-control-bg);
	}

	:global(.rh-menu__trigger--icon:hover),
	:global(.rh-menu__trigger--icon[data-state='open']) {
		border-color: var(--rh-color-border-strong);
		background-color: var(--rh-color-control-bg-hover);
	}

	:global(.rh-menu) {
		z-index: var(--rh-z-overlay);
		min-width: 220px;
		padding: var(--rh-space-1);
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-surface);
		box-shadow: var(--rh-shadow-menu);
	}

	:global(.rh-menu__item) {
		display: flex;
		align-items: center;
		gap: var(--rh-space-3);
		width: 100%;
		height: var(--rh-control-height);
		padding: 0 var(--rh-space-2);
		border: 0;
		border-radius: var(--rh-radius-sm);
		background: none;
		color: var(--rh-color-text);
		font-family: inherit;
		font-size: var(--rh-font-size-base);
		text-align: left;
		cursor: pointer;
	}

	:global(.rh-menu__item[data-highlighted]) {
		outline: none;
		background-color: var(--rh-color-hover);
	}

	:global(.rh-menu__item--danger) {
		color: var(--rh-color-danger-text);
	}

	:global(.rh-menu__item--danger[data-highlighted]) {
		background-color: var(--rh-color-danger-subtle);
	}

	:global(.rh-menu__item[data-disabled]) {
		opacity: 0.5;
	}

	:global(.rh-menu__check) {
		display: inline-flex;
		margin-left: auto;
		color: var(--rh-color-accent);
	}

	:global(.rh-menu__head) {
		padding: var(--rh-space-2);
	}

	:global(.rh-menu__label) {
		padding: var(--rh-space-1) var(--rh-space-2);
		color: var(--rh-color-text-subtle);
		font-size: var(--rh-font-size-xs);
		font-weight: var(--rh-font-weight-semibold);
	}

	:global(.rh-menu__separator) {
		height: 1px;
		margin: var(--rh-space-1) 0;
		border: 0;
		background-color: var(--rh-color-border-muted);
	}
</style>
