<script lang="ts">
	import type { HTMLInputAttributes } from 'svelte/elements';
	import { t } from '#lib/i18n/index.ts';
	import { plural } from '#lib/list-format.ts';
	import {
		MAX_PASSWORD_BYTES,
		MIN_PASSWORD_CHARS,
		passwordChars,
		passwordProblem
	} from '#lib/password-policy.ts';
	import Field from './Field.svelte';
	import Icon from './Icon.svelte';

	let {
		value = $bindable(''),
		label,
		error,
		id,
		submitted = false,
		required = true,
		class: className,
		...rest
	}: {
		value?: string;
		label?: string;
		/** An error of the server; it replaces the rule line. */
		error?: string;
		id?: string;
		/** The form had a submit attempt, so a broken rule shows as an error. */
		submitted?: boolean;
		required?: boolean;
	} & Omit<HTMLInputAttributes, 'type'> = $props();

	const uid = $props.id();
	const inputId = $derived(id ?? `rh-new-password-${uid}`);
	const ruleId = $derived(`${inputId}-rule`);
	const errorId = $derived(`${inputId}-error`);

	let visible = $state(false);
	let blurred = $state(false);

	const count = $derived(passwordChars(value));
	const problem = $derived(passwordProblem(value));
	const rule = $derived.by(() => {
		if (problem === 'too_long') return t('password.tooLong', { max: MAX_PASSWORD_BYTES });
		if (problem === null) return plural('password.enough', count);
		if (count === 0) return t('password.rule', { min: MIN_PASSWORD_CHARS });
		return t('password.progress', { count, min: MIN_PASSWORD_CHARS });
	});
	// Typing is not an error yet: the tone changes only after the person leaves the field or
	// submits. A too long value can never become valid by typing more, so it shows at once.
	const ruleError = $derived(
		problem === 'too_long' || (problem !== null && (blurred || submitted)) ? rule : undefined
	);
	const shownError = $derived(error ?? ruleError);
</script>

<Field {label} error={shownError} for={inputId} {required} {errorId}>
	<div class="rh-input-affix">
		<input
			id={inputId}
			type={visible ? 'text' : 'password'}
			bind:value
			class={['rh-input', className]}
			aria-invalid={shownError ? 'true' : undefined}
			aria-describedby={shownError ? errorId : ruleId}
			autocomplete="new-password"
			{required}
			{...rest}
			onblur={(event) => {
				if (value !== '') blurred = true;
				rest.onblur?.(event);
			}}
		/>
		<button
			type="button"
			class="rh-input-affix__button"
			aria-pressed={visible}
			aria-label={visible ? t('common.hide') : t('common.show')}
			onclick={() => (visible = !visible)}
		>
			<Icon name={visible ? 'eye-off' : 'eye'} />
		</button>
	</div>
	{#if !shownError}
		<p class="rh-field__help rule" id={ruleId}>
			{#if problem === null}<Icon name="check" size="xs" />{/if}{rule}
		</p>
	{/if}
</Field>

<style>
	.rule {
		display: flex;
		align-items: center;
		gap: var(--rh-space-1);
	}
</style>
