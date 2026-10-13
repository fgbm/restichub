<script lang="ts">
	import Button from './Button.svelte';
	import { t } from '#lib/i18n/index.ts';
	import '#lib/styles/controls.css';

	let {
		value,
		label,
		secret = false
	}: {
		value: string;
		label?: string;
		/** A value shown one time only, such as a token. */
		secret?: boolean;
	} = $props();

	const uid = $props.id();
	const inputId = `rh-copy-${uid}`;

	let copied = $state(false);

	async function copy(): Promise<void> {
		try {
			await navigator.clipboard.writeText(value);
			copied = true;
			setTimeout(() => {
				copied = false;
			}, 1500);
		} catch {
			copied = false;
		}
	}
</script>

<div class="rh-field">
	{#if label}
		<label class="rh-field__label" for={inputId}>{label}</label>
	{/if}
	<div class="copy-field" class:copy-field--secret={secret}>
		<input id={inputId} class="rh-input rh-input--mono" readonly {value} />
		<Button icon={copied ? 'check' : 'copy'} onclick={copy}>
			{copied ? t('common.copied') : t('common.copy')}
		</Button>
	</div>
</div>

<style>
	.copy-field {
		display: flex;
		align-items: stretch;
		gap: var(--rh-space-2);
	}

	.copy-field--secret :global(.rh-input) {
		background-color: var(--rh-color-canvas-subtle);
	}
</style>
