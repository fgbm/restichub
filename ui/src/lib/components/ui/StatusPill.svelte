<script lang="ts">
	import type { Snippet } from 'svelte';
	import Icon from './Icon.svelte';
	import type { IconName } from './icons';
	import type { StatusTone } from './types';

	let {
		tone = 'neutral',
		icon,
		children
	}: { tone?: StatusTone; icon?: IconName; children: Snippet } = $props();
</script>

<span class="status status--{tone}">
	{#if icon}<Icon name={icon} size="xs" />{/if}
	{@render children()}
</span>

<style>
	.status {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		height: 20px;
		padding: 0 var(--rh-space-2);
		border: var(--rh-border-width) solid var(--status-border, var(--rh-color-neutral-border));
		border-radius: var(--rh-radius-pill);
		background-color: var(--status-bg, var(--rh-color-neutral-bg));
		color: var(--status-text, var(--rh-color-neutral-text));
		font-size: var(--rh-font-size-xs);
		font-weight: var(--rh-font-weight-semibold);
		white-space: nowrap;
	}

	.status::before {
		content: '';
		flex: none;
		width: 6px;
		height: 6px;
		border-radius: 50%;
		background-color: currentColor;
	}

	.status--success {
		--status-bg: var(--rh-color-success-subtle);
		--status-border: var(--rh-color-success-subtle-border);
		--status-text: var(--rh-color-success-subtle-text);
	}

	.status--running {
		--status-bg: var(--rh-color-running-subtle);
		--status-border: var(--rh-color-running-subtle-border);
		--status-text: var(--rh-color-running-subtle-text);
	}

	.status--queued,
	.status--cancelled,
	.status--neutral {
		--status-bg: var(--rh-color-neutral-bg);
		--status-border: var(--rh-color-neutral-border);
		--status-text: var(--rh-color-neutral-text);
	}

	.status--warning {
		--status-bg: var(--rh-color-warning-subtle);
		--status-border: var(--rh-color-warning-subtle-border);
		--status-text: var(--rh-color-warning-subtle-text);
	}

	.status--failed {
		--status-bg: var(--rh-color-danger-subtle);
		--status-border: var(--rh-color-danger-subtle-border);
		--status-text: var(--rh-color-danger-subtle-text);
	}

	.status--info {
		--status-bg: var(--rh-color-info-subtle);
		--status-border: var(--rh-color-info-subtle-border);
		--status-text: var(--rh-color-info-subtle-text);
	}
</style>
