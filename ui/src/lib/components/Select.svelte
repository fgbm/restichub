<script lang="ts">
	import type { HTMLSelectAttributes } from 'svelte/elements';
	import type { SelectOption } from './types';

	let {
		value = $bindable(''),
		options = [],
		id,
		invalid = false,
		placeholder,
		...rest
	}: {
		value?: string;
		options?: SelectOption[];
		id?: string;
		invalid?: boolean;
		placeholder?: string;
	} & HTMLSelectAttributes = $props();
</script>

<select
	{id}
	bind:value
	class="select"
	class:select--invalid={invalid}
	aria-invalid={invalid ? 'true' : undefined}
	{...rest}
>
	{#if placeholder}
		<option value="" disabled>{placeholder}</option>
	{/if}
	{#each options as option (option.value)}
		<option value={option.value}>{option.label}</option>
	{/each}
</select>

<style>
	.select {
		width: 100%;
		padding: var(--rh-space-2) var(--rh-space-3);
		border: var(--rh-border-width) solid var(--rh-color-border-strong);
		border-radius: var(--rh-radius-sm);
		background-color: var(--rh-color-surface);
		color: var(--rh-color-text);
		font-family: var(--rh-font-sans);
		font-size: var(--rh-font-size-base);
		line-height: var(--rh-line-height-base);
	}

	.select:focus-visible {
		border-color: var(--rh-color-accent);
		outline: 2px solid var(--rh-color-focus);
		outline-offset: 0;
	}

	.select--invalid {
		border-color: var(--rh-color-danger);
	}

	.select:disabled {
		background-color: var(--rh-color-surface-muted);
		color: var(--rh-color-text-muted);
		cursor: not-allowed;
	}
</style>
