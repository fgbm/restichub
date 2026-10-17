<script lang="ts">
	import { Combobox as Primitive } from 'bits-ui';
	import { t } from '#lib/i18n/index.ts';
	import Field from './Field.svelte';
	import Icon from './Icon.svelte';
	import type { SelectOption } from './types';

	let {
		value = $bindable(''),
		options = [],
		label,
		help,
		error,
		id,
		required = false,
		disabled = false,
		placeholder,
		emptyLabel = 'No matches'
	}: {
		value?: string;
		options?: SelectOption[];
		label?: string;
		help?: string;
		error?: string;
		id?: string;
		required?: boolean;
		disabled?: boolean;
		placeholder?: string;
		emptyLabel?: string;
	} = $props();

	const uid = $props.id();
	const inputId = $derived(id ?? `rh-combobox-${uid}`);
	const helpId = $derived(`${inputId}-help`);
	const errorId = $derived(`${inputId}-error`);
	const describedBy = $derived(error ? errorId : help ? helpId : undefined);
	const emptyId = $derived(`${inputId}-empty`);

	let query = $state('');
	let touched = $state(false);
	let open = $state(false);

	const selectedLabel = $derived(options.find((option) => option.value === value)?.label ?? '');
	const showEmpty = $derived(disabled && selectedLabel === '');
	const shown = $derived(
		touched && query.trim() !== ''
			? options.filter((option) => option.label.toLowerCase().includes(query.trim().toLowerCase()))
			: options
	);

	function onOpenChange(open: boolean) {
		if (!open) {
			touched = false;
			query = '';
		}
	}
</script>

<Field {label} {help} {error} for={inputId} {required} {helpId} {errorId}>
	<Primitive.Root
		type="single"
		bind:value
		bind:open
		items={options}
		{disabled}
		{onOpenChange}
		inputValue={touched ? query : selectedLabel}
	>
		<div class="rh-combobox">
			<Primitive.Input
				id={inputId}
				class="rh-input rh-combobox__input"
				placeholder={disabled ? undefined : placeholder}
				aria-invalid={error ? 'true' : undefined}
				aria-describedby={showEmpty ? `${describedBy ?? ''} ${emptyId}`.trim() : describedBy}
				aria-required={required ? 'true' : undefined}
				onclick={() => (open = true)}
				oninput={(event) => {
					touched = true;
					query = event.currentTarget.value;
				}}
			/>
			<Primitive.Trigger
				class="rh-combobox__trigger"
				aria-label={t('common.showOptions')}
				tabindex={-1}
			>
				<Icon name="chevron-down" />
			</Primitive.Trigger>
			{#if showEmpty}
				<span class="rh-input-wrap__empty" id={emptyId}>{t('common.notSet')}</span>
			{/if}
		</div>
		<Primitive.Portal>
			<Primitive.Content class="rh-combobox__list" sideOffset={4}>
				<Primitive.Viewport>
					{#each shown as option (option.value)}
						<Primitive.Item class="rh-combobox__option" value={option.value} label={option.label}>
							{option.label}
							{#if option.value === value}
								<span class="rh-combobox__check"><Icon name="check" /></span>
							{/if}
						</Primitive.Item>
					{:else}
						<div class="rh-combobox__empty">{emptyLabel}</div>
					{/each}
				</Primitive.Viewport>
			</Primitive.Content>
		</Primitive.Portal>
	</Primitive.Root>
</Field>

<style>
	.rh-combobox {
		position: relative;
		display: block;
	}

	:global(.rh-combobox__input) {
		padding-right: var(--rh-space-8);
	}

	:global(.rh-combobox__trigger) {
		position: absolute;
		top: 0;
		right: 0;
		display: inline-flex;
		align-items: center;
		justify-content: center;
		width: var(--rh-control-height);
		height: var(--rh-control-height);
		padding: 0;
		border: 0;
		background: none;
		color: var(--rh-color-text-muted);
		cursor: pointer;
	}

	:global(.rh-combobox__list) {
		z-index: var(--rh-z-overlay);
		min-width: var(--bits-combobox-anchor-width);
		max-height: 280px;
		overflow-y: auto;
		padding: var(--rh-space-1);
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-surface);
		box-shadow: var(--rh-shadow-menu);
	}

	:global(.rh-combobox__option) {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: var(--rh-space-3);
		min-height: var(--rh-control-height);
		padding: 0 var(--rh-space-2);
		border-radius: var(--rh-radius-sm);
		color: var(--rh-color-text);
		font-size: var(--rh-font-size-base);
		cursor: pointer;
	}

	:global(.rh-combobox__option[data-highlighted]) {
		background-color: var(--rh-color-hover);
	}

	:global(.rh-combobox__option[data-disabled]) {
		opacity: 0.5;
	}

	:global(.rh-combobox__check) {
		display: inline-flex;
		color: var(--rh-color-accent);
	}

	:global(.rh-combobox__empty) {
		padding: var(--rh-space-2);
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}
</style>
