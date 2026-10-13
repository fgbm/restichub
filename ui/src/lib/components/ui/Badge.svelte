<script lang="ts">
	import type { Snippet } from 'svelte';
	import Icon from './Icon.svelte';
	import type { IconName } from './icons';
	import type { BadgeTone } from './types';

	let {
		tone = 'neutral',
		icon,
		children
	}: { tone?: BadgeTone; icon?: IconName; children: Snippet } = $props();
</script>

<span class="badge badge--{tone}">
	{#if icon}<Icon name={icon} size="xs" />{/if}
	{@render children()}
</span>

<style>
	.badge {
		display: inline-flex;
		align-items: center;
		gap: var(--rh-space-1);
		height: 20px;
		padding: 0 var(--rh-space-2);
		border: var(--rh-border-width) solid var(--rh-color-neutral-border);
		border-radius: var(--rh-radius-pill);
		background-color: var(--rh-color-neutral-bg);
		color: var(--rh-color-neutral-text);
		font-size: var(--rh-font-size-xs);
		font-weight: var(--rh-font-weight-semibold);
		white-space: nowrap;
	}

	.badge--accent {
		border-color: var(--rh-color-accent-subtle-border);
		background-color: var(--rh-color-accent-subtle);
		color: var(--rh-color-accent-subtle-text);
	}

	.badge--success {
		border-color: var(--rh-color-success-subtle-border);
		background-color: var(--rh-color-success-subtle);
		color: var(--rh-color-success-subtle-text);
	}

	.badge--warning {
		border-color: var(--rh-color-warning-subtle-border);
		background-color: var(--rh-color-warning-subtle);
		color: var(--rh-color-warning-subtle-text);
	}

	.badge--danger {
		border-color: var(--rh-color-danger-subtle-border);
		background-color: var(--rh-color-danger-subtle);
		color: var(--rh-color-danger-subtle-text);
	}

	.badge--info {
		border-color: var(--rh-color-info-subtle-border);
		background-color: var(--rh-color-info-subtle);
		color: var(--rh-color-info-subtle-text);
	}
</style>
