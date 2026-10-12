<script lang="ts">
	import type { Snippet } from 'svelte';

	type Tone = 'neutral' | 'accent' | 'success' | 'warning' | 'danger' | 'info';

	let { tone = 'neutral', children }: { tone?: Tone; children: Snippet } = $props();
</script>

<span class="badge badge--{tone}">{@render children()}</span>

<style>
	.badge {
		display: inline-flex;
		align-items: center;
		padding: 1px var(--rh-space-2);
		border: var(--rh-border-width) solid transparent;
		border-radius: var(--rh-radius-pill);
		font-size: var(--rh-font-size-xs);
		font-weight: var(--rh-font-weight-medium);
		line-height: var(--rh-line-height-tight);
		white-space: nowrap;
	}

	.badge--neutral {
		background-color: var(--rh-color-surface-muted);
		border-color: var(--rh-color-border);
		color: var(--rh-color-text-muted);
	}

	.badge--accent {
		background-color: var(--rh-color-accent-subtle);
		border-color: var(--rh-color-accent);
		color: var(--rh-color-accent-subtle-text);
	}

	.badge--success {
		background-color: var(--rh-color-success-subtle);
		border-color: var(--rh-color-success);
		color: var(--rh-color-success-subtle-text);
	}

	.badge--warning {
		background-color: var(--rh-color-warning-subtle);
		border-color: var(--rh-color-warning);
		color: var(--rh-color-warning-subtle-text);
	}

	.badge--danger {
		background-color: var(--rh-color-danger-subtle);
		border-color: var(--rh-color-danger);
		color: var(--rh-color-danger-subtle-text);
	}

	.badge--info {
		background-color: var(--rh-color-info-subtle);
		border-color: var(--rh-color-info);
		color: var(--rh-color-info-subtle-text);
	}
</style>
