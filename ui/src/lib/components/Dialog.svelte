<script lang="ts">
	import type { Snippet } from 'svelte';
	import { t } from '#lib/i18n/index.ts';
	import { focusTrap } from './focus-trap';

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
		children: Snippet;
		footer?: Snippet;
	} = $props();

	const id = $props.id();
	const titleId = `rh-dialog-title-${id}`;
	const descriptionId = `${titleId}-description`;

	function close(): void {
		open = false;
	}

	function onOverlayClick(event: MouseEvent): void {
		if (event.target === event.currentTarget) close();
	}

	function onKeydown(event: KeyboardEvent): void {
		if (event.key === 'Escape') {
			event.stopPropagation();
			close();
		}
	}
</script>

{#if open}
	<div class="dialog-overlay" role="presentation" onclick={onOverlayClick}>
		<div
			class="dialog"
			role="dialog"
			aria-modal="true"
			tabindex="-1"
			aria-labelledby={titleId}
			aria-describedby={description ? descriptionId : undefined}
			use:focusTrap
			onkeydown={onKeydown}
		>
			<header class="dialog__header">
				<h2 id={titleId} class="dialog__title">{title}</h2>
				<button type="button" class="dialog__close" onclick={close} aria-label={t('common.close')}>
					<span aria-hidden="true">✕</span>
				</button>
			</header>
			{#if description}
				<p id={descriptionId} class="dialog__description">{description}</p>
			{/if}
			<div class="dialog__body">{@render children()}</div>
			{#if footer}
				<div class="dialog__footer">{@render footer()}</div>
			{/if}
		</div>
	</div>
{/if}

<style>
	.dialog-overlay {
		position: fixed;
		inset: 0;
		z-index: var(--rh-z-overlay);
		display: flex;
		align-items: flex-start;
		justify-content: center;
		padding: var(--rh-space-10) var(--rh-space-4);
		background-color: var(--rh-color-overlay);
		overflow-y: auto;
	}

	.dialog {
		width: 100%;
		max-width: 30rem;
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-surface);
		box-shadow: var(--rh-shadow-overlay);
	}

	.dialog__header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: var(--rh-space-2);
		padding: var(--rh-space-3) var(--rh-space-4);
		border-bottom: var(--rh-border-width) solid var(--rh-color-border);
	}

	.dialog__title {
		margin: 0;
		font-size: var(--rh-font-size-lg);
		font-weight: var(--rh-font-weight-semibold);
	}

	.dialog__close {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		width: 28px;
		height: 28px;
		border: 0;
		border-radius: var(--rh-radius-sm);
		background: transparent;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-md);
		cursor: pointer;
	}

	.dialog__close:hover {
		background-color: var(--rh-color-hover);
		color: var(--rh-color-text);
	}

	.dialog__description {
		margin: 0;
		padding: var(--rh-space-3) var(--rh-space-4) 0;
		color: var(--rh-color-text-muted);
	}

	.dialog__body {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-4);
		padding: var(--rh-space-4);
	}

	.dialog__footer {
		display: flex;
		justify-content: flex-end;
		gap: var(--rh-space-2);
		padding: var(--rh-space-3) var(--rh-space-4);
		border-top: var(--rh-border-width) solid var(--rh-color-border);
	}
</style>
