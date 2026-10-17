<script lang="ts">
	import Switch from './Switch.svelte';

	let {
		checked = $bindable(false),
		label,
		help,
		disabled = false,
		id
	}: {
		checked?: boolean;
		label: string;
		help?: string;
		disabled?: boolean;
		id?: string;
	} = $props();

	const uid = $props.id();
	const switchId = $derived(id ?? `rh-switch-field-${uid}`);
</script>

<div class="switch-field">
	<div class="switch-field__text">
		<label class="switch-field__label" for={switchId}>{label}</label>
		{#if help}<p class="switch-field__help">{help}</p>{/if}
	</div>
	<Switch id={switchId} bind:checked {disabled} />
</div>

<style>
	.switch-field {
		display: flex;
		align-items: flex-start;
		justify-content: space-between;
		gap: var(--rh-space-4);
	}

	.switch-field__label {
		font-weight: var(--rh-font-weight-medium);
	}

	.switch-field__help {
		margin: 0;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}
</style>
