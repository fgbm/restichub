<script lang="ts">
	import type { HTMLInputAttributes } from 'svelte/elements';
	import Field from './Field.svelte';

	let {
		value = $bindable(''),
		label,
		help,
		error,
		id,
		type = 'text',
		mono = false,
		required = false,
		...rest
	}: {
		value?: string;
		label?: string;
		help?: string;
		error?: string;
		id?: string;
		type?: HTMLInputAttributes['type'];
		mono?: boolean;
		required?: boolean;
	} & HTMLInputAttributes = $props();

	const uid = $props.id();
	const inputId = $derived(id ?? `rh-input-${uid}`);
	const helpId = $derived(`${inputId}-help`);
	const errorId = $derived(`${inputId}-error`);
	const describedBy = $derived(error ? errorId : help ? helpId : undefined);
</script>

<Field {label} {help} {error} for={inputId} {required} {helpId} {errorId}>
	<input
		id={inputId}
		{type}
		bind:value
		class="rh-input"
		class:rh-input--mono={mono}
		aria-invalid={error ? 'true' : undefined}
		aria-describedby={describedBy}
		{required}
		{...rest}
	/>
</Field>
