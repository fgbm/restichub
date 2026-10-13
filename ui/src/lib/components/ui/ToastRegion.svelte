<script lang="ts">
	import { dismissToast, getToasts } from '#lib/stores/toasts.svelte.ts';
	import Toast from './Toast.svelte';

	const toasts = $derived(getToasts());
</script>

<div class="toast-region" aria-live="polite" aria-atomic="false">
	{#each toasts as toast (toast.id)}
		<Toast tone={toast.tone} onDismiss={() => dismissToast(toast.id)}>{toast.message}</Toast>
	{/each}
</div>

<style>
	.toast-region {
		position: fixed;
		right: var(--rh-space-6);
		bottom: var(--rh-space-6);
		z-index: var(--rh-z-toast);
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-2);
	}
</style>
