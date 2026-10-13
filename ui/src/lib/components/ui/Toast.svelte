<script lang="ts">
	import type { Snippet } from 'svelte';
	import { t } from '#lib/i18n/index.ts';

	type Tone = 'info' | 'success' | 'danger';

	let {
		tone = 'info',
		title,
		onDismiss,
		children
	}: {
		tone?: Tone;
		title?: string;
		onDismiss?: () => void;
		children?: Snippet;
	} = $props();
</script>

<div class="toast toast--{tone}" role="status">
	<div class="toast__body">
		{#if title}
			<p class="toast__title">{title}</p>
		{/if}
		{#if children}
			<div class="toast__text">{@render children()}</div>
		{/if}
	</div>
	<button
		type="button"
		class="toast__close"
		aria-label={t('common.dismiss')}
		onclick={() => onDismiss?.()}
	>
		✕
	</button>
</div>

<style>
	.toast {
		display: flex;
		align-items: flex-start;
		gap: var(--rh-space-3);
		min-width: 280px;
		max-width: 420px;
		padding: var(--rh-space-3) var(--rh-space-4);
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-surface);
		box-shadow: var(--rh-shadow-overlay);
	}

	.toast--success {
		border-left: 3px solid var(--rh-color-success);
	}

	.toast--danger {
		border-left: 3px solid var(--rh-color-danger);
	}

	.toast--info {
		border-left: 3px solid var(--rh-color-info);
	}

	.toast__body {
		flex: 1 1 auto;
		min-width: 0;
	}

	.toast__title {
		margin: 0;
		font-weight: var(--rh-font-weight-semibold);
	}

	.toast__text {
		margin-top: 2px;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.toast__close {
		border: 0;
		background: transparent;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
		cursor: pointer;
	}
</style>
