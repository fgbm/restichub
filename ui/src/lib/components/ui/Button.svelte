<script lang="ts">
	import type { Snippet } from 'svelte';
	import type { HTMLButtonAttributes } from 'svelte/elements';
	import Icon from './Icon.svelte';
	import type { IconName } from './icons';

	type Variant = 'primary' | 'secondary' | 'danger' | 'ghost' | 'link';
	type Size = 'sm' | 'md';

	let {
		variant = 'secondary',
		size = 'md',
		loading = false,
		disabled = false,
		icon,
		full = false,
		type = 'button',
		onclick,
		children,
		class: className,
		...rest
	}: {
		variant?: Variant;
		size?: Size;
		loading?: boolean;
		disabled?: boolean;
		/** Leading icon. Without children the button is icon-only and needs `aria-label`. */
		icon?: IconName;
		full?: boolean;
		type?: HTMLButtonAttributes['type'];
		onclick?: (event: MouseEvent) => void;
		children?: Snippet;
	} & HTMLButtonAttributes = $props();

	const iconOnly = $derived(Boolean(icon) && !children);
</script>

<button
	{type}
	class={['btn', `btn--${variant}`, `btn--${size}`, className]}
	class:btn--icon={iconOnly}
	class:btn--full={full}
	disabled={disabled || loading}
	aria-busy={loading ? 'true' : undefined}
	{onclick}
	{...rest}
>
	{#if loading}
		<span class="btn__spinner" aria-hidden="true"></span>
	{:else if icon}
		<Icon name={icon} size={iconOnly && size === 'sm' ? 'xs' : 'md'} />
	{/if}
	{#if children}
		<span class="btn__label">{@render children()}</span>
	{/if}
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

	.btn--link {
		height: auto;
		padding: 0;
		border-color: transparent;
		background: none;
		color: var(--rh-color-link);
		font-weight: var(--rh-font-weight-regular);
	}

	.btn--link:hover:not(:disabled),
	.btn--link:active:not(:disabled) {
		border-color: transparent;
		background: none;
		color: var(--rh-color-link-hover);
		text-decoration: underline;
	}

	.btn--icon {
		width: var(--rh-control-height);
		padding: 0;
	}

	.btn--icon.btn--sm {
		width: 28px;
	}

	.btn--full {
		width: 100%;
	}

	.btn:disabled {
		opacity: 0.5;
		cursor: not-allowed;
	}

	.btn[aria-busy='true']:disabled {
		opacity: 1;
		cursor: progress;
	}

	.btn__spinner {
		flex: none;
		width: 12px;
		height: 12px;
		border: 2px solid var(--rh-color-border);
		border-top-color: currentColor;
		border-radius: 50%;
		animation: btn-spin 0.7s linear infinite;
	}

	@keyframes btn-spin {
		to {
			transform: rotate(360deg);
		}
	}
</style>
