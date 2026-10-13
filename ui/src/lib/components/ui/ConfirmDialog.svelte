<script lang="ts">
	import { t } from '#lib/i18n/index.ts';
	import Button from './Button.svelte';
	import Dialog from './Dialog.svelte';

	let {
		open = $bindable(false),
		title,
		message,
		confirmLabel,
		cancelLabel = t('common.cancel'),
		tone = 'primary',
		loading = false,
		onConfirm
	}: {
		open?: boolean;
		title: string;
		message: string;
		confirmLabel: string;
		cancelLabel?: string;
		tone?: 'primary' | 'danger';
		loading?: boolean;
		onConfirm?: () => void;
	} = $props();
</script>

<Dialog bind:open {title} description={message} icon={tone === 'danger' ? 'i-octagon' : undefined}>
	{#snippet footer()}
		<Button variant="secondary" onclick={() => (open = false)}>{cancelLabel}</Button>
		<Button variant={tone} {loading} onclick={() => onConfirm?.()}>{confirmLabel}</Button>
	{/snippet}
</Dialog>
