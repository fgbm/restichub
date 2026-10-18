<script lang="ts">
	import { Combobox as Primitive } from 'bits-ui';
	import { tick } from 'svelte';
	import { t } from '#lib/i18n/index.ts';
	import Field from './Field.svelte';
	import Icon from './Icon.svelte';
	import RecordLine from './RecordLine.svelte';
	import { matchesRecord } from './record';
	import type { RecordOption, RecordPart } from './types';

	let {
		value = $bindable(''),
		options = [],
		label,
		ariaLabel,
		help,
		error,
		id,
		required = false,
		disabled = false,
		placeholder,
		searchPlaceholder,
		emptyLabel,
		allLabel,
		countLabel,
		onchange
	}: {
		value?: string;
		options?: RecordOption[];
		label?: string;
		/** The accessible name when the field has no visible label, for example in a filter row. */
		ariaLabel?: string;
		help?: string;
		error?: string;
		id?: string;
		required?: boolean;
		disabled?: boolean;
		placeholder?: string;
		searchPlaceholder?: string;
		/** The text of the list when the search matches nothing. */
		emptyLabel?: string;
		/** Adds a first option with the empty value, for example "All" in a filter. */
		allLabel?: string;
		/** The count of the options in the footer of the list, for example "4 hosts". */
		countLabel?: (count: number) => string;
		onchange?: (value: string) => void;
	} = $props();

	const uid = $props.id();
	const fieldId = $derived(id ?? `rh-record-${uid}`);
	const labelId = $derived(`${fieldId}-label`);
	const valueId = $derived(`${fieldId}-value`);
	const detailId = $derived(`${fieldId}-detail`);
	const listId = $derived(`${fieldId}-list`);
	const helpId = $derived(`${fieldId}-help`);
	const errorId = $derived(`${fieldId}-error`);

	let open = $state(false);
	let query = $state('');
	let trigger = $state<HTMLButtonElement | null>(null);
	let panel = $state<HTMLDivElement | null>(null);

	const all = $derived<RecordOption[]>(
		allLabel === undefined ? options : [{ value: '', label: allLabel }, ...options]
	);
	const selected = $derived(all.find((option) => option.value === value));
	const shown = $derived(
		all.filter((option) => matchesRecord(option.label, option.description, query, option.search))
	);
	const first = $derived(selected?.description?.[0]);
	const detailParts = $derived.by<RecordPart[] | undefined>(() => {
		const detail = selected?.detail;
		if (Array.isArray(detail)) return detail;
		if (typeof detail === 'string') return [detail];
		return first !== undefined ? [first] : undefined;
	});
	const closedText = $derived(selected?.label ?? value);
	const showNotSet = $derived(disabled && closedText === '');
	const found = $derived(shown.filter((option) => option.value !== '' || allLabel === undefined));
	const footCount = $derived(
		query.trim() !== ''
			? t('recordPicker.shown', { shown: found.length, total: options.length })
			: (countLabel?.(options.length) ?? '')
	);
	const describedBy = $derived(
		[error ? errorId : help ? helpId : '', detailParts ? detailId : ''].filter(Boolean).join(' ') ||
			undefined
	);

	function setOpen(next: boolean): void {
		if (disabled && next) return;
		open = next;
		if (!next) closed();
	}

	// The search field is gone after a close, so the focus goes back to the field.
	async function closed(): Promise<void> {
		query = '';
		await tick();
		const active = document.activeElement;
		if (!active || active === document.body || panel?.contains(active)) trigger?.focus();
	}

	function onTriggerKeydown(event: KeyboardEvent): void {
		if (disabled || open) return;
		if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
			event.preventDefault();
			setOpen(true);
		} else if (event.key.length === 1 && event.key !== ' ' && !event.ctrlKey && !event.metaKey) {
			event.preventDefault();
			query = event.key;
			setOpen(true);
		}
	}

	function focusOnMount(node: HTMLInputElement): void {
		node.focus({ preventScroll: true });
		node.setSelectionRange(node.value.length, node.value.length);
	}
</script>

<Field {label} {help} {error} for={fieldId} {required} {helpId} {errorId} {labelId}>
	<Primitive.Root
		type="single"
		bind:value={
			() => value,
			(next) => {
				value = next;
				onchange?.(next);
			}
		}
		bind:open={() => open, setOpen}
		items={all}
		{disabled}
		allowDeselect={false}
	>
		<button
			bind:this={trigger}
			id={fieldId}
			type="button"
			class="rh-select rh-record__field"
			{disabled}
			role="combobox"
			aria-haspopup="listbox"
			aria-expanded={open}
			aria-controls={open ? listId : undefined}
			aria-labelledby={label ? `${labelId} ${valueId}` : undefined}
			aria-label={label ? undefined : [ariaLabel, closedText].filter(Boolean).join(', ')}
			aria-describedby={describedBy}
			aria-invalid={error ? 'true' : undefined}
			aria-required={required ? 'true' : undefined}
			onclick={() => setOpen(!open)}
			onkeydown={onTriggerKeydown}
		>
			{#if showNotSet}
				<span class="rh-record__empty" id={valueId}>{t('common.notSet')}</span>
			{:else if closedText === ''}
				<span class="rh-record__placeholder" id={valueId}>{placeholder ?? ''}</span>
			{:else}
				{#if selected?.dot}
					<span class="rh-record__dot rh-record__dot--{selected.dot}" aria-hidden="true"></span>
				{/if}
				<span class="rh-record__value" id={valueId}>{closedText}</span>
				{#if detailParts}
					<span class="rh-record__detail" id={detailId}>
						· <RecordLine parts={detailParts} />
					</span>
				{/if}
			{/if}
			<span class="rh-select-wrap__icon"><Icon name="chevron-down" /></span>
		</button>
		<Primitive.Portal>
			<Primitive.Content
				id={listId}
				customAnchor={trigger}
				sideOffset={4}
				onInteractOutside={(event) => {
					const target = event.target as Node | null;
					if (target && (trigger?.contains(target) || panel?.contains(target)))
						event.preventDefault();
				}}
			>
				{#snippet child({ props, wrapperProps })}
					<div {...wrapperProps}>
						<div class="rh-record__panel" bind:this={panel}>
							<div class="rh-record__search">
								<span class="rh-record__search-icon"><Icon name="search" /></span>
								<Primitive.Input
									aria-label={t('filter.facetSearch', { name: label ?? ariaLabel ?? '' })}
									oninput={(event) => (query = event.currentTarget.value)}
								>
									{#snippet child({ props: inputProps })}
										<input
											{...inputProps}
											class="rh-input rh-record__search-input"
											type="search"
											autocomplete="off"
											placeholder={searchPlaceholder ?? t('common.search')}
											value={query}
											{@attach focusOnMount}
										/>
									{/snippet}
								</Primitive.Input>
							</div>
							<div {...props} class="rh-record__list" aria-label={label ?? ariaLabel}>
								<Primitive.Viewport>
									{#each shown as option, index (option.value)}
										{@const optionId = `${fieldId}-o${index}`}
										<Primitive.Item
											class="rh-record__option"
											value={option.value}
											label={option.label}
											disabled={option.disabled}
											aria-labelledby={optionId}
											aria-describedby={[
												option.description?.length ? `${optionId}-d` : '',
												option.state && option.value !== value ? `${optionId}-s` : '',
												option.reason ? `${optionId}-r` : ''
											]
												.filter(Boolean)
												.join(' ') || undefined}
										>
											{#if option.dot}
												<span class="rh-record__dot rh-record__dot--{option.dot}" aria-hidden="true"
												></span>
											{/if}
											<span class="rh-record__option-text">
												<span class="rh-record__option-label" id={optionId}>
													{option.label}
												</span>
												{#if option.description?.length}
													<RecordLine parts={option.description} id="{optionId}-d" />
												{/if}
												{#if option.reason}
													<span class="rh-record__reason" id="{optionId}-r">{option.reason}</span>
												{/if}
											</span>
											{#if option.value === value}
												<span class="rh-record__check"><Icon name="check" /></span>
											{:else if option.state}
												<span class="rh-record__state" id="{optionId}-s">{option.state}</span>
											{/if}
										</Primitive.Item>
									{:else}
										<div class="rh-record__none">{emptyLabel ?? t('filter.noMatches')}</div>
									{/each}
								</Primitive.Viewport>
							</div>
							<div class="rh-record__foot">
								<span role="status">{footCount}</span>
								<span class="rh-record__keys">{t('recordPicker.keys')}</span>
							</div>
						</div>
					</div>
				{/snippet}
			</Primitive.Content>
		</Primitive.Portal>
	</Primitive.Root>
</Field>

<style>
	:global(.rh-record__field) {
		position: relative;
		display: flex;
		align-items: center;
		gap: var(--rh-space-1);
		min-width: 0;
		text-align: left;
		cursor: pointer;
	}

	:global(.rh-record__field:disabled) {
		cursor: not-allowed;
	}

	.rh-record__value,
	.rh-record__detail {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.rh-record__value {
		flex: none;
		max-width: 60%;
	}

	.rh-record__detail {
		min-width: 0;
		color: var(--rh-color-text-muted);
	}

	.rh-record__placeholder {
		color: var(--rh-color-text-subtle);
	}

	/* Stays muted and italic, so an empty locked field does not read as a stored value. */
	.rh-record__empty {
		color: var(--rh-color-text-subtle);
		font-style: italic;
	}

	.rh-record__panel {
		display: flex;
		flex-direction: column;
		min-width: max(var(--bits-floating-anchor-width), 18rem);
		max-width: min(32rem, var(--bits-floating-available-width));
		max-height: 340px;
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-surface);
		box-shadow: var(--rh-shadow-menu);
	}

	.rh-record__search {
		position: relative;
		padding: var(--rh-space-2);
		border-bottom: var(--rh-border-width) solid var(--rh-color-border-muted);
	}

	.rh-record__search-icon {
		position: absolute;
		top: 50%;
		left: calc(var(--rh-space-2) + var(--rh-space-3));
		display: inline-flex;
		transform: translateY(-50%);
		color: var(--rh-color-text-muted);
		pointer-events: none;
	}

	:global(.rh-record__search-input) {
		padding-left: var(--rh-space-8);
	}

	/* The floating wrapper copies the z-index of this element. The list opens after a dialog in the
	   document, so the same layer as the dialog puts it on top. */
	:global(.rh-record__list) {
		position: relative;
		z-index: var(--rh-z-panel);
		overflow-y: auto;
		padding: var(--rh-space-1);
	}

	:global(.rh-record__option) {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: var(--rh-space-3);
		padding: var(--rh-space-1) var(--rh-space-2);
		border-radius: var(--rh-radius-sm);
		color: var(--rh-color-text);
		font-size: var(--rh-font-size-base);
		cursor: pointer;
	}

	:global(.rh-record__option[data-selected]) {
		background-color: var(--rh-color-selected);
	}

	:global(.rh-record__option[data-highlighted]) {
		background-color: var(--rh-color-hover);
	}

	:global(.rh-record__option[data-disabled]) {
		cursor: not-allowed;
	}

	:global(.rh-record__option[data-disabled]) .rh-record__option-label,
	:global(.rh-record__option[data-disabled]) .rh-record__dot,
	:global(.rh-record__option[data-disabled]) .rh-record__state,
	:global(.rh-record__option[data-disabled]) :global(.record-line) {
		opacity: 0.55;
	}

	.rh-record__option-text {
		display: flex;
		flex: 1;
		flex-direction: column;
		gap: 2px;
		min-width: 0;
	}

	.rh-record__option-label {
		overflow: hidden;
		font-weight: var(--rh-font-weight-medium);
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.rh-record__dot {
		flex: none;
		width: 8px;
		height: 8px;
		border-radius: var(--rh-radius-pill);
	}

	.rh-record__option .rh-record__dot {
		align-self: flex-start;
		margin-top: 6px;
	}

	.rh-record__dot--success {
		background-color: var(--rh-color-success);
	}

	.rh-record__dot--warning {
		background-color: var(--rh-color-warning);
	}

	.rh-record__dot--neutral {
		background-color: var(--rh-color-text-subtle);
	}

	.rh-record__state {
		flex: none;
		align-self: flex-start;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
		white-space: nowrap;
	}

	.rh-record__reason {
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.rh-record__check {
		display: inline-flex;
		flex: none;
		color: var(--rh-color-accent);
	}

	.rh-record__foot {
		display: flex;
		justify-content: space-between;
		gap: var(--rh-space-3);
		padding: var(--rh-space-1) var(--rh-space-3);
		border-top: var(--rh-border-width) solid var(--rh-color-border-muted);
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-xs);
	}

	.rh-record__keys {
		white-space: nowrap;
	}

	.rh-record__none {
		padding: var(--rh-space-2);
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}
</style>
