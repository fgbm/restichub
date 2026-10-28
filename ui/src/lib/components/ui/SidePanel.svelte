<script lang="ts">
	import type { Snippet } from 'svelte';
	import { Dialog as Primitive } from 'bits-ui';
	import { t } from '#lib/i18n/index.ts';
	import Icon from './Icon.svelte';

	let {
		open = $bindable(false),
		title,
		subtitle,
		children,
		footer
	}: {
		open?: boolean;
		title: string;
		subtitle?: string;
		children: Snippet;
		footer?: Snippet;
	} = $props();
</script>

<Primitive.Root bind:open>
	<Primitive.Portal>
		<Primitive.Overlay class="rh-panel-overlay" />
		<Primitive.Content class="rh-panel">
			<header class="rh-panel__head">
				<div class="rh-panel__heading">
					<Primitive.Title class="rh-panel__title">{title}</Primitive.Title>
					{#if subtitle}
						<Primitive.Description class="rh-panel__subtitle">{subtitle}</Primitive.Description>
					{/if}
				</div>
				<Primitive.Close class="rh-panel__close" aria-label={t('common.close')}>
					<Icon name="x" size="xs" />
				</Primitive.Close>
			</header>
			<div class="rh-panel__body">{@render children()}</div>
			{#if footer}
				<div class="rh-panel__foot">{@render footer()}</div>
			{/if}
		</Primitive.Content>
	</Primitive.Portal>
</Primitive.Root>

<style>
	:global(.rh-panel-overlay) {
		position: fixed;
		inset: 0;
		z-index: var(--rh-z-overlay);
		background-color: var(--rh-color-overlay);
	}

	:global(.rh-panel) {
		position: fixed;
		top: 0;
		/* The scroll lock hides the window scrollbar while the panel is open, so the panel reaches
		   the window edge. */
		right: 0;
		z-index: var(--rh-z-panel);
		display: flex;
		flex-direction: column;
		width: 100%;
		max-width: 480px;
		height: 100vh;
		border-left: var(--rh-border-width) solid var(--rh-color-border);
		background-color: var(--rh-color-canvas);
		color: var(--rh-color-text);
		box-shadow: var(--rh-shadow-overlay);
	}

	:global(.rh-panel__head) {
		display: flex;
		align-items: center;
		gap: var(--rh-space-3);
		padding: var(--rh-space-3) var(--rh-space-4);
		border-bottom: var(--rh-border-width) solid var(--rh-color-border);
	}

	:global(.rh-panel__heading) {
		flex: 1;
		min-width: 0;
	}

	:global(.rh-panel__title) {
		margin: 0;
		line-height: var(--rh-line-height-base);
		font-size: var(--rh-font-size-lg);
		font-weight: var(--rh-font-weight-semibold);
	}

	:global(.rh-panel__subtitle) {
		margin: var(--rh-space-1) 0 0;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	:global(.rh-panel__close) {
		display: inline-flex;
		flex: none;
		align-items: center;
		justify-content: center;
		width: 28px;
		height: 28px;
		padding: 0;
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-control-bg);
		color: var(--rh-color-text);
		cursor: pointer;
		transition:
			background-color var(--rh-transition-fast),
			border-color var(--rh-transition-fast);
	}

	:global(.rh-panel__close:hover) {
		background-color: var(--rh-color-control-bg-hover);
		border-color: var(--rh-color-border-strong);
	}

	:global(.rh-panel:focus-visible) {
		outline: none;
	}

	:global(.rh-panel__body) {
		flex: 1;
		overflow-y: auto;
		padding: var(--rh-space-4);
	}

	:global(.rh-panel__foot) {
		display: flex;
		align-items: center;
		gap: var(--rh-space-2);
		padding: var(--rh-space-3) var(--rh-space-4);
		border-top: var(--rh-border-width) solid var(--rh-color-border);
	}
</style>
