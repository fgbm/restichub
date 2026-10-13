<script lang="ts">
	import type { Snippet } from 'svelte';
	import { t } from '#lib/i18n/index.ts';
	import Icon from './Icon.svelte';
	import type { IconName } from './icons';

	type Tone = 'info' | 'success' | 'attention' | 'danger';

	let {
		tone = 'info',
		variant = 'banner',
		title,
		icon,
		code,
		requestId,
		actions,
		children
	}: {
		tone?: Tone;
		/** `banner` has a title and a text; `inline` is one short line in a form. */
		variant?: 'banner' | 'inline';
		title: string;
		icon?: IconName;
		code?: string;
		requestId?: string;
		actions?: Snippet;
		children?: Snippet;
	} = $props();

	const toneIcons: Record<Tone, IconName> = {
		info: 'info',
		success: 'check-circle',
		attention: 'alert',
		danger: 'octagon'
	};
	const inlineIcons: Record<Tone, IconName> = {
		info: 'info',
		success: 'check-circle',
		attention: 'alert',
		danger: 'alert'
	};

	const iconName = $derived(icon ?? (variant === 'inline' ? inlineIcons[tone] : toneIcons[tone]));
</script>

<div class="alert alert--{variant} alert--{tone}" role={tone === 'danger' ? 'alert' : 'status'}>
	<span class="alert__icon">
		<Icon name={iconName} size={variant === 'inline' ? 'xs' : 'lg'} />
	</span>
	<div class="alert__body">
		<p class="alert__title">{title}</p>
		{#if children}
			<div class="alert__text">{@render children()}</div>
		{/if}
		{#if code || requestId}
			<p class="alert__meta">
				{#if code}
					<span>{t('alert.code')}: <code>{code}</code></span>
				{/if}
				{#if requestId}
					<span>{t('alert.requestId')}: <code>{requestId}</code></span>
				{/if}
			</p>
		{/if}
	</div>
	{#if actions}
		<div class="alert__actions">{@render actions()}</div>
	{/if}
</div>

<style>
	.alert {
		display: flex;
		align-items: flex-start;
		gap: var(--rh-space-3);
		padding: var(--rh-space-3) var(--rh-space-4);
		border: var(--rh-border-width) solid var(--alert-border, var(--rh-color-border));
		border-radius: var(--rh-radius-md);
		background-color: var(--alert-bg, var(--rh-color-canvas-subtle));
		color: var(--rh-color-text);
	}

	.alert__icon {
		display: inline-flex;
		flex: none;
		margin-top: 1px;
		color: var(--alert-accent, var(--rh-color-text-muted));
	}

	.alert__body {
		flex: 1 1 auto;
		min-width: 0;
	}

	.alert__actions {
		display: flex;
		flex: none;
		align-items: center;
		gap: var(--rh-space-2);
	}

	.alert--banner.alert--info {
		--alert-bg: var(--rh-color-info-subtle);
		--alert-border: var(--rh-color-info-subtle-border);
		--alert-accent: var(--rh-color-info);
	}

	.alert--banner.alert--success {
		--alert-bg: var(--rh-color-success-subtle);
		--alert-border: var(--rh-color-success-subtle-border);
		--alert-accent: var(--rh-color-success);
	}

	.alert--banner.alert--attention {
		--alert-bg: var(--rh-color-warning-subtle);
		--alert-border: var(--rh-color-warning-subtle-border);
		--alert-accent: var(--rh-color-warning);
	}

	.alert--banner.alert--danger {
		--alert-bg: var(--rh-color-danger-subtle);
		--alert-border: var(--rh-color-danger-subtle-border);
		--alert-accent: var(--rh-color-danger);
	}

	.alert__title {
		margin: 0;
		font-weight: var(--rh-font-weight-semibold);
	}

	.alert__text {
		margin-top: 2px;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.alert__meta {
		display: flex;
		flex-wrap: wrap;
		gap: var(--rh-space-4);
		margin: var(--rh-space-2) 0 0;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.alert__meta code {
		padding: 1px 5px;
		border: var(--rh-border-width) solid var(--rh-color-border-muted);
		border-radius: var(--rh-radius-sm);
		background-color: var(--rh-color-code-bg);
		color: var(--rh-color-text);
	}

	.alert--inline {
		gap: var(--rh-space-2);
		padding: var(--rh-space-3);
		font-size: var(--rh-font-size-sm);
	}

	.alert--inline .alert__icon {
		color: inherit;
	}

	.alert--inline .alert__body {
		display: flex;
		flex-wrap: wrap;
		align-items: baseline;
		gap: var(--rh-space-1) var(--rh-space-2);
	}

	.alert--inline .alert__title {
		font-weight: var(--rh-font-weight-regular);
	}

	.alert--inline .alert__meta {
		margin: 0;
		color: inherit;
	}

	.alert--inline.alert--danger {
		--alert-bg: var(--rh-color-danger-subtle);
		--alert-border: var(--rh-color-danger-subtle-border);
		color: var(--rh-color-danger-text);
	}

	.alert--inline.alert--success {
		--alert-bg: var(--rh-color-success-subtle);
		--alert-border: var(--rh-color-success-subtle-border);
		color: var(--rh-color-success-text);
	}

	.alert--inline.alert--attention {
		--alert-bg: var(--rh-color-warning-subtle);
		--alert-border: var(--rh-color-warning-subtle-border);
		color: var(--rh-color-warning-text);
	}
</style>
