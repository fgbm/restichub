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
	disabled={disabled || loading}
	aria-busy={loading ? 'true' : undefined}
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
		height: var(--rh-control-height);
		padding: 0 var(--rh-space-3);
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-control-bg);
		color: var(--rh-color-text);
		font-family: var(--rh-font-sans);
		font-size: var(--rh-font-size-base);
		font-weight: var(--rh-font-weight-semibold);
		line-height: 1;
		white-space: nowrap;
		cursor: pointer;
		transition:
			background-color var(--rh-transition-fast),
			border-color var(--rh-transition-fast),
			color var(--rh-transition-fast);
	}

	.btn:hover:not(:disabled) {
		background-color: var(--rh-color-control-bg-hover);
		border-color: var(--rh-color-border-strong);
	}

	.btn:active:not(:disabled) {
		background-color: var(--rh-color-control-bg-active);
	}

	.btn--sm {
		height: 28px;
		padding: 0 var(--rh-space-2);
		font-size: var(--rh-font-size-sm);
	}

	.btn--primary {
		background-color: var(--rh-color-accent);
		border-color: var(--rh-color-accent-active);
		color: var(--rh-color-text-on-accent);
	}

	.btn--primary:hover:not(:disabled) {
		background-color: var(--rh-color-accent-hover);
		border-color: var(--rh-color-accent-active);
	}

	.btn--primary:active:not(:disabled) {
		background-color: var(--rh-color-accent-active);
	}

	.btn--danger {
		background-color: var(--rh-color-danger);
		border-color: var(--rh-color-danger-hover);
		color: var(--rh-color-text-inverse);
	}

	.btn--danger:hover:not(:disabled) {
		background-color: var(--rh-color-danger-hover);
		border-color: var(--rh-color-danger-hover);
	}

	.btn--ghost {
		background-color: transparent;
		border-color: transparent;
		color: var(--rh-color-text-muted);
	}

	.btn--ghost:hover:not(:disabled) {
		background-color: var(--rh-color-hover);
		border-color: transparent;
		color: var(--rh-color-text);
	}

	.btn:disabled {
		opacity: 0.5;
		cursor: not-allowed;
	}

	.btn__spinner {
		width: 0.9em;
		height: 0.9em;
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
