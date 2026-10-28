<script lang="ts">
	import type { Snippet } from 'svelte';
	import { Dialog as Primitive } from 'bits-ui';
	import { t } from '#lib/i18n/index.ts';
	import Icon from './Icon.svelte';
	import type { IconName } from './icons';

	type Size = 'sm' | 'md' | 'lg' | 'xl';

	let {
		open = $bindable(false),
		title,
		description,
		size = 'md',
		icon,
		closable = true,
		onsubmit,
		children,
		footer
	}: {
		open?: boolean;
		title: string;
		description?: string;
		/** Width: `sm` 400px, `md` 440px, `lg` 480px, `xl` 920px. */
		size?: Size;
		/** Title icon; a destructive dialog uses `octagon`. */
		icon?: IconName;
		/** Shows the close button in the header. Escape closes the dialog in any case. */
		closable?: boolean;
		/** Makes the body and the footer one form, so Enter submits it. */
		onsubmit?: (event: SubmitEvent) => void;
		children?: Snippet;
		footer?: Snippet;
	} = $props();

	function submit(event: SubmitEvent): void {
		event.preventDefault();
		onsubmit?.(event);
	}
</script>

{#snippet content()}
	{#if description || children}
		<div class="rh-dialog__body">
			{#if description}
				<Primitive.Description class="rh-dialog__description">{description}</Primitive.Description>
			{/if}
			{@render children?.()}
		</div>
	{/if}
	{#if footer}
		<div class="rh-dialog__foot">{@render footer()}</div>
	{/if}
{/snippet}

<Primitive.Root bind:open>
	<Primitive.Portal>
		<Primitive.Overlay class="rh-dialog-overlay" />
		<Primitive.Content class="rh-dialog rh-dialog--{size}">
			<header class="rh-dialog__head">
				<Primitive.Title class="rh-dialog__title" level={2}>
					{#if icon}<Icon name={icon} />{/if}
					{title}
				</Primitive.Title>
				{#if closable}
					<Primitive.Close class="rh-dialog__close" aria-label={t('common.close')}>
						<Icon name="x" size="xs" />
					</Primitive.Close>
				{/if}
			</header>
			{#if onsubmit}
				<form class="rh-dialog__form" onsubmit={submit} novalidate>{@render content()}</form>
			{:else}
				{@render content()}
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
		/* The middle of the screen. A mobile page that is wider than the screen makes the percentage
		   boxes of a fixed layer wider than the screen, and `50%` then puts the dialog off it. */
		left: 50vw;
		z-index: var(--rh-z-panel);
		display: flex;
		flex-direction: column;
		width: min(440px, calc(100vw - 32px));
		max-height: calc(100vh - 32px);
		transform: translate(-50%, -50%);
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-lg);
		background-color: var(--rh-color-canvas);
		color: var(--rh-color-text);
		box-shadow: var(--rh-shadow-overlay);
	}

	:global(.rh-dialog--sm) {
		width: min(400px, calc(100vw - 32px));
	}

	:global(.rh-dialog--lg) {
		width: min(480px, calc(100vw - 32px));
	}

	:global(.rh-dialog--xl) {
		width: min(920px, calc(100vw - 32px));
	}

	:global(.rh-dialog:focus-visible) {
		outline: none;
	}

	:global(.rh-dialog__head) {
		display: flex;
		align-items: center;
		gap: var(--rh-space-3);
		padding: var(--rh-space-4) var(--rh-space-4) var(--rh-space-3);
	}

	:global(.rh-dialog__title) {
		display: flex;
		align-items: center;
		gap: var(--rh-space-2);
		margin: 0;
		font-size: var(--rh-font-size-lg);
		font-weight: var(--rh-font-weight-semibold);
		line-height: var(--rh-line-height-tight);
	}

	:global(.rh-dialog__title .icon) {
		color: var(--rh-color-text-muted);
	}

	:global(.rh-dialog__title .icon[data-icon='octagon']) {
		color: var(--rh-color-danger);
	}

	:global(.rh-dialog__close) {
		display: inline-flex;
		flex: none;
		align-items: center;
		justify-content: center;
		width: 28px;
		height: 28px;
		margin-left: auto;
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

	:global(.rh-dialog__close:hover) {
		background-color: var(--rh-color-control-bg-hover);
		border-color: var(--rh-color-border-strong);
	}

	:global(.rh-dialog__form) {
		display: contents;
	}

	:global(.rh-dialog__body) {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-4);
		min-height: 0;
		overflow-y: auto;
		padding: 0 var(--rh-space-4) var(--rh-space-4);
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-base);
	}

	:global(.rh-dialog__body p) {
		margin: 0;
	}

	:global(.rh-dialog__description) {
		margin: 0;
	}

	:global(.rh-dialog__foot) {
		display: flex;
		flex: none;
		align-items: center;
		justify-content: flex-end;
		gap: var(--rh-space-2);
		padding: var(--rh-space-3) var(--rh-space-4);
		border-top: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: 0 0 var(--rh-radius-lg) var(--rh-radius-lg);
		background-color: var(--rh-color-canvas-subtle);
	}
</style>
