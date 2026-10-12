<script lang="ts">
	import { t } from '#lib/i18n/index.ts';
	import { dismissToast, getToasts } from '#lib/stores/toasts.svelte.ts';

	const toasts = $derived(getToasts());
</script>

<div class="toasts" aria-live="polite" aria-atomic="false">
	{#each toasts as toast (toast.id)}
		<div class="toast toast--{toast.tone}" role="status">
			<span class="toast__message">{toast.message}</span>
			<button
				type="button"
				class="toast__close"
				onclick={() => dismissToast(toast.id)}
				aria-label={t('common.dismiss')}
			>
				<span aria-hidden="true">✕</span>
			</button>
		</div>
	{/each}
</div>

<style>
	.toasts {
		position: fixed;
		right: var(--rh-space-4);
		bottom: var(--rh-space-4);
		z-index: var(--rh-z-toast);
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-2);
		max-width: 24rem;
	}

	.toast {
		display: flex;
		align-items: flex-start;
		justify-content: space-between;
		gap: var(--rh-space-3);
		padding: var(--rh-space-2) var(--rh-space-3);
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-left-width: 3px;
		border-radius: var(--rh-radius-sm);
		background-color: var(--rh-color-surface);
		box-shadow: var(--rh-shadow-overlay);
		font-size: var(--rh-font-size-base);
	}

	.toast--info {
		border-left-color: var(--rh-color-info);
	}

	.toast--success {
		border-left-color: var(--rh-color-success);
	}

	.toast--warning {
		border-left-color: var(--rh-color-warning);
	}

	.toast--danger {
		border-left-color: var(--rh-color-danger);
	}

	.toast__message {
		color: var(--rh-color-text);
	}

	.toast__close {
		border: 0;
		background: transparent;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
		cursor: pointer;
	}
</style>
