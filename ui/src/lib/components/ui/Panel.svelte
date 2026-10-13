<script lang="ts">
	import type { Snippet } from 'svelte';

	let {
		title,
		subtitle,
		header,
		footer,
		children
	}: {
		title?: string;
		subtitle?: string;
		header?: Snippet;
		footer?: Snippet;
		children: Snippet;
	} = $props();
</script>

<section class="panel">
	{#if header}
		<div class="panel__head">{@render header()}</div>
	{:else if title}
		<div class="panel__head">
			<div>
				<h2 class="panel__title">{title}</h2>
				{#if subtitle}
					<p class="panel__subtitle">{subtitle}</p>
				{/if}
			</div>
		</div>
	{/if}
	<div class="panel__body">{@render children()}</div>
	{#if footer}
		<div class="panel__foot">{@render footer()}</div>
	{/if}
</section>

<style>
	.panel {
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-canvas);
		overflow: hidden;
	}

	.panel__head {
		display: flex;
		align-items: center;
		gap: var(--rh-space-3);
		padding: var(--rh-space-3) var(--rh-space-4);
		border-bottom: var(--rh-border-width) solid var(--rh-color-border);
		background-color: var(--rh-color-canvas-subtle);
	}

	.panel__title {
		margin: 0;
		font-size: var(--rh-font-size-base);
		font-weight: var(--rh-font-weight-semibold);
	}

	.panel__subtitle {
		margin: var(--rh-space-1) 0 0;
		font-size: var(--rh-font-size-sm);
		color: var(--rh-color-text-muted);
	}

	.panel__body {
		padding: var(--rh-space-4);
	}

	.panel__foot {
		display: flex;
		align-items: center;
		gap: var(--rh-space-2);
		padding: var(--rh-space-3) var(--rh-space-4);
		border-top: var(--rh-border-width) solid var(--rh-color-border);
	}
</style>
