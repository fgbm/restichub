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
		/* As in the top bar: `100%` ends at the window scrollbar, and the reserved width keeps the
		   right edge of the toasts in place when a long page adds the scrollbar. */
		right: calc(var(--rh-space-6) + var(--rh-scrollbar-width) - (100vw - 100%));
		bottom: var(--rh-space-6);
		z-index: var(--rh-z-toast);
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-2);
	}
</style>
