<script lang="ts">
	import type { Snippet } from 'svelte';

	let {
		label,
		hint,
		error,
		for: htmlFor,
		required = false,
		children
	}: {
		label: string;
		hint?: string;
		error?: string;
		for?: string;
		required?: boolean;
		children: Snippet;
	} = $props();
</script>

<div class="form-field" class:form-field--invalid={Boolean(error)}>
	<label class="form-field__label" for={htmlFor}>
		{label}
		{#if required}
			<span class="form-field__required" aria-hidden="true">*</span>
		{/if}
	</label>
	{@render children()}
	{#if error}
		<p class="form-field__error" role="alert">{error}</p>
	{:else if hint}
		<p class="form-field__hint">{hint}</p>
	{/if}
</div>

<style>
	.form-field {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-1);
	}

	.form-field__label {
		font-size: var(--rh-font-size-sm);
		font-weight: var(--rh-font-weight-medium);
		color: var(--rh-color-text);
	}

	.form-field__required {
		margin-inline-start: var(--rh-space-1);
		color: var(--rh-color-danger);
	}

	.form-field__hint {
		margin: 0;
		font-size: var(--rh-font-size-sm);
		color: var(--rh-color-text-subtle);
	}

	.form-field__error {
		margin: 0;
		font-size: var(--rh-font-size-sm);
		color: var(--rh-color-danger);
	}
</style>
