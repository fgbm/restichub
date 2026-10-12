<script lang="ts">
	import type { Snippet } from 'svelte';
	import type { HTMLButtonAttributes } from 'svelte/elements';

	type Variant = 'primary' | 'secondary' | 'danger' | 'ghost';
	type Size = 'sm' | 'md';

	let {
		variant = 'secondary',
		size = 'md',
		loading = false,
		disabled = false,
		type = 'button',
		onclick,
		children,
		...rest
	}: {
		variant?: Variant;
		size?: Size;
		loading?: boolean;
		disabled?: boolean;
		type?: HTMLButtonAttributes['type'];
		onclick?: (event: MouseEvent) => void;
		children: Snippet;
	} & HTMLButtonAttributes = $props();
</script>

<button
	{type}
	class="btn btn--{variant} btn--{size}"
	class:btn--loading={loading}
	disabled={disabled || loading}
	aria-busy={loading}
	{onclick}
	{...rest}
>
	{#if loading}
		<span class="btn__spinner" aria-hidden="true"></span>
	{/if}
	<span class="btn__label">{@render children()}</span>
</button>

<style>
	.btn {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		gap: var(--rh-space-2);
		border: var(--rh-border-width) solid transparent;
		border-radius: var(--rh-radius-sm);
		font-family: var(--rh-font-sans);
		font-size: var(--rh-font-size-base);
		font-weight: var(--rh-font-weight-medium);
		line-height: var(--rh-line-height-tight);
		cursor: pointer;
		white-space: nowrap;
		transition:
			background-color var(--rh-transition-fast),
			border-color var(--rh-transition-fast),
			color var(--rh-transition-fast);
	}

	.btn--sm {
		padding: var(--rh-space-1) var(--rh-space-3);
		font-size: var(--rh-font-size-sm);
	}

	.btn--md {
		padding: var(--rh-space-2) var(--rh-space-4);
	}

	.btn--primary {
		background-color: var(--rh-color-accent);
		border-color: var(--rh-color-accent);
		color: var(--rh-color-accent-text);
	}

	.btn--primary:hover:not(:disabled) {
		background-color: var(--rh-color-accent-hover);
		border-color: var(--rh-color-accent-hover);
	}

	.btn--primary:active:not(:disabled) {
		background-color: var(--rh-color-accent-active);
	}

	.btn--secondary {
		background-color: var(--rh-color-surface);
		border-color: var(--rh-color-border-strong);
		color: var(--rh-color-text);
	}

	.btn--secondary:hover:not(:disabled) {
		background-color: var(--rh-color-surface-muted);
	}

	.btn--danger {
		background-color: var(--rh-color-danger);
		border-color: var(--rh-color-danger);
		color: var(--rh-color-danger-text);
	}

	.btn--danger:hover:not(:disabled) {
		background-color: var(--rh-color-danger-hover);
		border-color: var(--rh-color-danger-hover);
	}

	.btn--ghost {
		background-color: transparent;
		border-color: transparent;
		color: var(--rh-color-text);
	}

	.btn--ghost:hover:not(:disabled) {
		background-color: var(--rh-color-hover);
	}

	.btn:disabled {
		opacity: 0.55;
		cursor: not-allowed;
	}

	.btn__spinner {
		width: 0.85em;
		height: 0.85em;
		border: 2px solid currentColor;
		border-right-color: transparent;
		border-radius: 50%;
		animation: btn-spin 0.7s linear infinite;
	}

	@keyframes btn-spin {
		to {
			transform: rotate(360deg);
		}
	}
</style>
