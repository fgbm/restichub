<script lang="ts">
	import type { HTMLInputAttributes } from 'svelte/elements';

	let {
		value = $bindable(''),
		id,
		type = 'text',
		invalid = false,
		mono = false,
		...rest
	}: {
		value?: string;
		id?: string;
		type?: HTMLInputAttributes['type'];
		invalid?: boolean;
		mono?: boolean;
	} & HTMLInputAttributes = $props();
</script>

<input
	{id}
	{type}
	bind:value
	class="field"
	class:field--mono={mono}
	class:field--invalid={invalid}
	aria-invalid={invalid ? 'true' : undefined}
	{...rest}
/>

<style>
	.field {
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

	.field--mono {
		font-family: var(--rh-font-mono);
		font-size: var(--rh-font-size-sm);
	}

	.field::placeholder {
		color: var(--rh-color-text-subtle);
	}

	.field:focus-visible {
		border-color: var(--rh-color-accent);
		outline: 2px solid var(--rh-color-focus);
		outline-offset: 0;
	}

	.field--invalid {
		border-color: var(--rh-color-danger);
	}

	.field:disabled {
		background-color: var(--rh-color-surface-muted);
		color: var(--rh-color-text-muted);
		cursor: not-allowed;
	}
</style>
