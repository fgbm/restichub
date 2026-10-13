<script lang="ts">
	import Button from './Button.svelte';
	import { t } from '#lib/i18n/index.ts';

	let { value, label }: { value: string; label?: string } = $props();

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

<div class="copy-field">
	{#if label}
		<span class="copy-field__label">{label}</span>
	{/if}
	<div class="copy-field__row">
		<code class="copy-field__value">{value}</code>
		<Button size="sm" onclick={copy}>{copied ? t('common.copied') : t('common.copy')}</Button>
	</div>
</div>

<style>
	.copy-field {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-1);
	}

	.copy-field__label {
		font-size: var(--rh-font-size-sm);
		font-weight: var(--rh-font-weight-semibold);
		color: var(--rh-color-text);
	}

	.copy-field__row {
		display: flex;
		align-items: center;
		gap: var(--rh-space-2);
	}

	.copy-field__value {
		flex: 1;
		min-width: 0;
		padding: var(--rh-space-2) var(--rh-space-3);
		border: var(--rh-border-width) solid var(--rh-color-border-strong);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-canvas-subtle);
		color: var(--rh-color-text);
		font-family: var(--rh-font-mono);
		font-size: var(--rh-font-size-sm);
		overflow-x: auto;
		white-space: nowrap;
	}
</style>
