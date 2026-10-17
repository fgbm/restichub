<script lang="ts">
	import type { Snippet } from 'svelte';
	import '#lib/styles/controls.css';
	import Icon from './Icon.svelte';

	let {
		label,
		help,
		error,
		for: htmlFor,
		required = false,
		helpId,
		errorId,
		labelId,
		children
	}: {
		label?: string;
		help?: string;
		error?: string;
		for?: string;
		required?: boolean;
		helpId?: string;
		errorId?: string;
		labelId?: string;
		children: Snippet;
	} = $props();
</script>

<div class="rh-field" class:rh-field--invalid={Boolean(error)}>
	{#if label}
		<label class="rh-field__label" for={htmlFor} id={labelId}>
			{label}
			{#if required}<span class="rh-field__req" aria-hidden="true">*</span>{/if}
		</label>
	{/if}
	{@render children()}
	{#if error}
		<p class="rh-field__error" id={errorId} role="alert">
			<Icon name="alert" size="xs" />{error}
		</p>
	{:else if help}
		<p class="rh-field__help" id={helpId}>{help}</p>
	{/if}
</div>
