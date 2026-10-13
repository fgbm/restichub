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
		required = false,
		...rest
	}: {
		value?: string;
		label?: string;
		help?: string;
		error?: string;
		id?: string;
		required?: boolean;
	} & HTMLInputAttributes = $props();

	const uid = $props.id();
	const inputId = $derived(id ?? `rh-password-${uid}`);
	const helpId = $derived(`${inputId}-help`);
	const errorId = $derived(`${inputId}-error`);
	const describedBy = $derived(error ? errorId : help ? helpId : undefined);

	let visible = $state(false);
</script>

<Field {label} {help} {error} for={inputId} {required} {helpId} {errorId}>
	<div class="rh-input-affix">
		<input
			id={inputId}
			type={visible ? 'text' : 'password'}
			bind:value
			class="rh-input"
			aria-invalid={error ? 'true' : undefined}
			aria-describedby={describedBy}
			{required}
			{...rest}
		/>
		<button
			type="button"
			class="rh-input-affix__button"
			aria-pressed={visible}
			onclick={() => (visible = !visible)}
		>
			{visible ? t('common.hide') : t('common.show')}
		</button>
	</div>
</Field>
