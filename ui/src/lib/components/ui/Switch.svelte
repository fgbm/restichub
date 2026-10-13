<script lang="ts">
	import { Switch } from 'bits-ui';
	import '#lib/styles/controls.css';

	let {
		checked = $bindable(false),
		label,
		disabled = false,
		id,
		...rest
	}: {
		checked?: boolean;
		label?: string;
		disabled?: boolean;
		id?: string;
	} = $props();

	const uid = $props.id();
	const switchId = $derived(id ?? `rh-switch-${uid}`);
</script>

<span class="rh-switch">
	<Switch.Root
		id={switchId}
		bind:checked
		{disabled}
		aria-label={label}
		class="rh-switch__track"
		{...rest}
	>
		<Switch.Thumb class="rh-switch__thumb" />
	</Switch.Root>
	{#if label}<span class="rh-switch__label">{label}</span>{/if}
</span>
