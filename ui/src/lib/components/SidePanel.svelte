<script lang="ts">
	import type { Snippet } from 'svelte';
	import { t } from '#lib/i18n/index.ts';
	import { focusTrap } from './focus-trap';

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

	const id = $props.id();
	const titleId = `rh-panel-title-${id}`;

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
	<div class="panel-overlay" role="presentation" onclick={onOverlayClick}>
		<div
			class="panel"
			role="dialog"
			aria-modal="true"
			tabindex="-1"
			aria-labelledby={titleId}
			use:focusTrap
			onkeydown={onKeydown}
		>
			<header class="panel__header">
				<div>
					<h2 id={titleId} class="panel__title">{title}</h2>
					{#if subtitle}
						<p class="panel__subtitle">{subtitle}</p>
					{/if}
				</div>
				<button type="button" class="panel__close" onclick={close} aria-label={t('common.close')}>
					<span aria-hidden="true">✕</span>
				</button>
			</header>
			<div class="panel__body">{@render children()}</div>
			{#if footer}
				<div class="panel__footer">{@render footer()}</div>
			{/if}
		</div>
	</div>
{/if}

<style>
	.panel-overlay {
		position: fixed;
		inset: 0;
		z-index: var(--rh-z-panel);
		display: flex;
		justify-content: flex-end;
		background-color: var(--rh-color-overlay);
	}

	.panel {
		display: flex;
		flex-direction: column;
		width: 100%;
		max-width: 32rem;
		height: 100%;
		border-left: var(--rh-border-width) solid var(--rh-color-border);
		background-color: var(--rh-color-surface);
		box-shadow: var(--rh-shadow-overlay);
	}

	.panel__header {
		display: flex;
		align-items: flex-start;
		justify-content: space-between;
		gap: var(--rh-space-2);
		padding: var(--rh-space-3) var(--rh-space-4);
		border-bottom: var(--rh-border-width) solid var(--rh-color-border);
	}

	.panel__title {
		margin: 0;
		font-size: var(--rh-font-size-lg);
		font-weight: var(--rh-font-weight-semibold);
	}

	.panel__subtitle {
		margin: var(--rh-space-1) 0 0;
		font-size: var(--rh-font-size-sm);
		color: var(--rh-color-text-muted);
	}

	.panel__close {
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

	.panel__close:hover {
		background-color: var(--rh-color-hover);
		color: var(--rh-color-text);
	}

	.panel__body {
		flex: 1;
		overflow-y: auto;
		padding: var(--rh-space-4);
	}

	.panel__footer {
		display: flex;
		justify-content: flex-end;
		gap: var(--rh-space-2);
		padding: var(--rh-space-3) var(--rh-space-4);
		border-top: var(--rh-border-width) solid var(--rh-color-border);
	}
</style>
