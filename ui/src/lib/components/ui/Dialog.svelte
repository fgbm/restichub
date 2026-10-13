<script lang="ts">
	import type { Snippet } from 'svelte';
	import { Dialog as Primitive } from 'bits-ui';
	import { t } from '#lib/i18n/index.ts';

	let {
		open = $bindable(false),
		title,
		description,
		children,
		footer
	}: {
		open?: boolean;
		title: string;
		description?: string;
		children?: Snippet;
		footer?: Snippet;
	} = $props();
</script>

<Primitive.Root bind:open>
	<Primitive.Portal>
		<Primitive.Overlay class="rh-dialog-overlay" />
		<Primitive.Content class="rh-dialog">
			<header class="rh-dialog__head">
				<Primitive.Title class="rh-dialog__title">{title}</Primitive.Title>
				<Primitive.Close class="rh-dialog__close" aria-label={t('common.close')}>✕</Primitive.Close>
			</header>
			{#if description}
				<Primitive.Description class="rh-dialog__description">{description}</Primitive.Description>
			{/if}
			{#if children}
				<div class="rh-dialog__body">{@render children()}</div>
			{/if}
			{#if footer}
				<div class="rh-dialog__footer">{@render footer()}</div>
			{/if}
		</Primitive.Content>
	</Primitive.Portal>
</Primitive.Root>

<style>
	:global(.rh-dialog-overlay) {
		position: fixed;
		inset: 0;
		z-index: var(--rh-z-overlay);
		background-color: var(--rh-color-overlay);
	}

	:global(.rh-dialog) {
		position: fixed;
		top: 50%;
		left: 50%;
		z-index: var(--rh-z-panel);
		width: min(440px, calc(100vw - 32px));
		transform: translate(-50%, -50%);
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-lg);
		background-color: var(--rh-color-canvas);
		color: var(--rh-color-text);
		box-shadow: var(--rh-shadow-overlay);
	}

	:global(.rh-dialog__head) {
		display: flex;
		align-items: center;
		gap: var(--rh-space-3);
		padding: var(--rh-space-4) var(--rh-space-4) var(--rh-space-3);
	}

	:global(.rh-dialog__title) {
		margin: 0;
		font-size: var(--rh-font-size-lg);
		font-weight: var(--rh-font-weight-semibold);
	}

	:global(.rh-dialog__close) {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		margin-left: auto;
		width: 28px;
		height: 28px;
		border: 0;
		border-radius: var(--rh-radius-md);
		background: transparent;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-base);
		cursor: pointer;
	}

	:global(.rh-dialog__close:hover) {
		background-color: var(--rh-color-hover);
		color: var(--rh-color-text);
	}

	:global(.rh-dialog__description) {
		margin: 0;
		padding: 0 var(--rh-space-4);
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-base);
	}

	:global(.rh-dialog__body) {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-4);
		padding: var(--rh-space-4);
	}

	:global(.rh-dialog__footer) {
		display: flex;
		align-items: center;
		justify-content: flex-end;
		gap: var(--rh-space-2);
		padding: var(--rh-space-3) var(--rh-space-4);
		border-top: var(--rh-border-width) solid var(--rh-color-border);
		background-color: var(--rh-color-canvas-subtle);
		border-radius: 0 0 var(--rh-radius-lg) var(--rh-radius-lg);
	}
</style>
