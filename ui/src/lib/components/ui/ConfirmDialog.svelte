<script lang="ts">
	import { t } from '#lib/i18n/index.ts';
	import Button from './Button.svelte';
	import Dialog from './Dialog.svelte';
	import type { IconName } from './icons';

	let {
		open = $bindable(false),
		title,
		message,
		confirmLabel,
		cancelLabel = t('common.cancel'),
		tone = 'primary',
		icon,
		loading = false,
		onConfirm
	}: {
		open?: boolean;
		title: string;
		message: string;
		confirmLabel: string;
		cancelLabel?: string;
		tone?: 'primary' | 'danger';
		/** Title icon; a destructive confirmation uses `octagon`. */
		icon?: IconName;
		loading?: boolean;
		onConfirm?: () => void;
	} = $props();
</script>

<Dialog bind:open {title} {icon} description={message} size="sm" closable={false}>
	{#snippet footer()}
		<Button variant="secondary" onclick={() => (open = false)}>{cancelLabel}</Button>
		<Button variant={tone} {loading} onclick={() => onConfirm?.()}>{confirmLabel}</Button>
	{/snippet}
</Dialog>
