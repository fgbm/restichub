<script lang="ts">
	import type { HTMLInputAttributes } from 'svelte/elements';

	let {
		checked = $bindable(false),
		id,
		label,
		...rest
	}: {
		checked?: boolean;
		id?: string;
		label?: string;
	} & HTMLInputAttributes = $props();
</script>

<label class="checkbox" for={id}>
	<input {id} type="checkbox" bind:checked {...rest} class="checkbox__input" />
	{#if label}<span class="checkbox__label">{label}</span>{/if}
</label>

<style>
	.checkbox {
		display: inline-flex;
		align-items: center;
		gap: var(--rh-space-2);
		font-size: var(--rh-font-size-base);
		cursor: pointer;
	}

	.checkbox__input {
		width: 16px;
		height: 16px;
		accent-color: var(--rh-color-accent);
		cursor: pointer;
	}

	.checkbox__label {
		color: var(--rh-color-text);
	}
</style>
