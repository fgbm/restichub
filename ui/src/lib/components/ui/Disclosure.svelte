<script lang="ts">
	import type { Snippet } from 'svelte';
	import Icon from './Icon.svelte';

	let {
		label,
		open = $bindable(false),
		hint,
		children
	}: {
		label: string;
		open?: boolean;
		/** A short state after the label, for example "all snapshots". */
		hint?: string;
		children?: Snippet;
	} = $props();

	const bodyId = $props.id();
</script>

<div class="rh-disclosure">
	<button
		type="button"
		class="rh-disclosure__toggle"
		aria-expanded={open}
		aria-controls={bodyId}
		onclick={() => (open = !open)}
	>
		<span class="rh-disclosure__chevron" class:rh-disclosure__chevron--open={open}>
			<Icon name="chevron-right" size="xs" />
		</span>
		{label}
		{#if hint}<span class="rh-disclosure__hint">— {hint}</span>{/if}
	</button>
	{#if open}
		<div id={bodyId} class="rh-disclosure__body">{@render children?.()}</div>
	{/if}
</div>

<style>
	.rh-disclosure {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-3);
	}

	.rh-disclosure__toggle {
		display: inline-flex;
		align-items: center;
		gap: var(--rh-space-1);
		align-self: flex-start;
		padding: 0;
		border: 0;
		background: none;
		color: var(--rh-color-link);
		font: inherit;
		cursor: pointer;
	}

	.rh-disclosure__toggle:hover {
		color: var(--rh-color-link-hover);
	}

	.rh-disclosure__toggle:focus-visible {
		outline: 2px solid var(--rh-color-focus);
		outline-offset: 2px;
		border-radius: var(--rh-radius-sm);
	}

	.rh-disclosure__chevron {
		display: inline-flex;
		transition: transform 120ms ease;
	}

	.rh-disclosure__chevron--open {
		transform: rotate(90deg);
	}

	.rh-disclosure__hint {
		color: var(--rh-color-text-muted);
	}

	@media (prefers-reduced-motion: reduce) {
		.rh-disclosure__chevron {
			transition: none;
		}
	}
</style>
