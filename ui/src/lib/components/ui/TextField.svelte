<script lang="ts">
	import type { HTMLInputAttributes } from 'svelte/elements';
	import { t } from '#lib/i18n/index.ts';
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
		disabled = false,
		readonly = false,
		placeholder,
		class: className,
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
	const emptyId = $derived(`${inputId}-empty`);
	// A locked field never shows the example placeholder: it would read as a stored value.
	const locked = $derived(Boolean(disabled || readonly));
	const showEmpty = $derived(locked && (value === '' || value == null));
	const describedBy = $derived(
		[error ? errorId : help ? helpId : undefined, showEmpty ? emptyId : undefined]
			.filter(Boolean)
			.join(' ') || undefined
	);
</script>

<Field {label} {help} {error} for={inputId} {required} {helpId} {errorId}>
	<div class="rh-input-wrap">
		<input
			id={inputId}
			{type}
			bind:value
			class={['rh-input', className]}
			class:rh-input--mono={mono}
			aria-invalid={error ? 'true' : undefined}
			aria-describedby={describedBy}
			placeholder={locked ? undefined : placeholder}
			{disabled}
			{readonly}
			{required}
			{...rest}
		/>
		{#if showEmpty}
			<span class="rh-input-wrap__empty" id={emptyId}>{t('common.notSet')}</span>
		{/if}
	</div>
</Field>
