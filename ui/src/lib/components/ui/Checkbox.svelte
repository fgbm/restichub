<script lang="ts">
	import type { HTMLInputAttributes } from 'svelte/elements';
	import '#lib/styles/controls.css';

	let {
		checked = $bindable(false),
		indeterminate = $bindable(false),
		label,
		help,
		id,
		...rest
	}: {
		checked?: boolean;
		indeterminate?: boolean;
		label?: string;
		help?: string;
		id?: string;
	} & HTMLInputAttributes = $props();

	const uid = $props.id();
	const inputId = $derived(id ?? `rh-checkbox-${uid}`);
	const helpId = $derived(`${inputId}-help`);
</script>

<label class="rh-checkbox" for={inputId}>
	<input
		id={inputId}
		type="checkbox"
		bind:checked
		bind:indeterminate
		aria-describedby={help ? helpId : undefined}
		{...rest}
	/>
	{#if label || help}
		<span>
			{#if label}<span class="rh-checkbox__label">{label}</span>{/if}
			{#if help}<span class="rh-checkbox__help" id={helpId}>{help}</span>{/if}
		</span>
	{/if}
</label>
