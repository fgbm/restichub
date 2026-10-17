<script lang="ts">
	import type { HTMLInputAttributes } from 'svelte/elements';
	import { t } from '#lib/i18n/index.ts';
	import Field from './Field.svelte';
	import Icon from './Icon.svelte';
	import type { IconName } from './icons';

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
		action,
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
		/** An icon button at the end of the field, for example a reset to the default value. */
		action?: { icon: IconName; label: string; onclick: () => void };
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
	<div class={action && !locked ? 'rh-input-affix' : 'rh-input-wrap'}>
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
		{#if action && !locked}
			<button
				type="button"
				class="rh-input-affix__button"
				title={action.label}
				aria-label={action.label}
				onclick={action.onclick}
			>
				<Icon name={action.icon} />
			</button>
		{/if}
	</div>
</Field>
