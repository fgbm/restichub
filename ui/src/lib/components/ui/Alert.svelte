<script lang="ts">
	import type { Snippet } from 'svelte';

	type Tone = 'info' | 'success' | 'attention' | 'danger';

	let {
		tone = 'info',
		title,
		children
	}: { tone?: Tone; title?: string; children?: Snippet } = $props();
</script>

<div class="alert alert--{tone}" role={tone === 'danger' ? 'alert' : 'status'}>
	<div class="alert__body">
		{#if title}
			<p class="alert__title">{title}</p>
		{/if}
		{#if children}
			<div class="alert__text">{@render children()}</div>
		{/if}
	</div>
</div>

<style>
	.alert {
		display: flex;
		align-items: flex-start;
		gap: var(--rh-space-2);
		padding: var(--rh-space-3) var(--rh-space-4);
		border: var(--rh-border-width) solid var(--alert-border, var(--rh-color-border));
		border-radius: var(--rh-radius-md);
		background-color: var(--alert-bg, var(--rh-color-canvas-subtle));
		color: var(--rh-color-text);
		font-size: var(--rh-font-size-base);
	}

	.alert--info {
		--alert-bg: var(--rh-color-info-subtle);
		--alert-border: var(--rh-color-info-subtle-border);
	}

	.alert--success {
		--alert-bg: var(--rh-color-success-subtle);
		--alert-border: var(--rh-color-success-subtle-border);
	}

	.alert--attention {
		--alert-bg: var(--rh-color-warning-subtle);
		--alert-border: var(--rh-color-warning-subtle-border);
	}

	.alert--danger {
		--alert-bg: var(--rh-color-danger-subtle);
		--alert-border: var(--rh-color-danger-subtle-border);
	}

	.alert__title {
		margin: 0;
		font-weight: var(--rh-font-weight-semibold);
	}

	.alert__text {
		margin-top: var(--rh-space-1);
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}
</style>
