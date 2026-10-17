<script lang="ts">
	import type { HTMLSelectAttributes } from 'svelte/elements';
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
		placeholder,
		disabled = false,
		class: className,
		...rest
	}: {
		value?: string;
		options?: SelectOption[];
		label?: string;
		help?: string;
		error?: string;
		id?: string;
		required?: boolean;
		placeholder?: string;
	} & HTMLSelectAttributes = $props();

	const uid = $props.id();
	const selectId = $derived(id ?? `rh-select-${uid}`);
	const helpId = $derived(`${selectId}-help`);
	const errorId = $derived(`${selectId}-error`);
	const describedBy = $derived(error ? errorId : help ? helpId : undefined);
	const emptyLabel = $derived.by(() => {
		if (!disabled) return placeholder;
		const empty = !value && !options.some((option) => option.value === '');
		return empty ? t('common.notSet') : undefined;
	});
</script>

<Field {label} {help} {error} for={selectId} {required} {helpId} {errorId}>
	<div class="rh-select-wrap">
		<select
			id={selectId}
			bind:value
			class={['rh-select', className]}
			aria-invalid={error ? 'true' : undefined}
			aria-describedby={describedBy}
			{disabled}
			{required}
			{...rest}
		>
			{#if emptyLabel}
				<option value="" disabled>{emptyLabel}</option>
			{/if}
			{#each options as option (option.value)}
				<option value={option.value}>{option.label}</option>
			{/each}
		</select>
		<span class="rh-select-wrap__icon"><Icon name="chevron-down" /></span>
	</div>
</Field>
