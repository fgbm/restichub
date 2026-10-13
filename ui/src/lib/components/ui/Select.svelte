<script lang="ts">
	import type { HTMLSelectAttributes } from 'svelte/elements';
	import Field from './Field.svelte';
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
</script>

<Field {label} {help} {error} for={selectId} {required} {helpId} {errorId}>
	<div class="rh-select-wrap">
		<select
			id={selectId}
			bind:value
			class="rh-select"
			aria-invalid={error ? 'true' : undefined}
			aria-describedby={describedBy}
			{required}
			{...rest}
		>
			{#if placeholder}
				<option value="" disabled>{placeholder}</option>
			{/if}
			{#each options as option (option.value)}
				<option value={option.value}>{option.label}</option>
			{/each}
		</select>
		<span class="rh-select-wrap__icon" aria-hidden="true">▾</span>
	</div>
</Field>
