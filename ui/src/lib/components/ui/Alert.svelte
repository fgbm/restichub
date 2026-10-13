<script lang="ts">
	import type { Snippet } from 'svelte';
	import { t } from '#lib/i18n/index.ts';
	import Icon from './Icon.svelte';
	import type { IconName } from './icons';

	type Tone = 'info' | 'success' | 'attention' | 'danger';

	const toneIcon: Record<Tone, IconName> = {
		info: 'i-info',
		success: 'i-check-circle',
		attention: 'i-alert',
		danger: 'i-octagon'
	};

	let {
		tone = 'info',
		title,
		icon,
		code,
		requestId,
		children
	}: {
		tone?: Tone;
		title: string;
		icon?: IconName;
		code?: string;
		requestId?: string;
		children?: Snippet;
	} = $props();
</script>

<div class="alert alert--{tone}" role={tone === 'danger' ? 'alert' : 'status'}>
	<Icon name={icon ?? toneIcon[tone]} size="lg" />
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
		--alert-accent: var(--rh-color-info);
	}

	.alert--success {
		--alert-bg: var(--rh-color-success-subtle);
		--alert-border: var(--rh-color-success-subtle-border);
		--alert-accent: var(--rh-color-success);
	}

	.alert--attention {
		--alert-bg: var(--rh-color-warning-subtle);
		--alert-border: var(--rh-color-warning-subtle-border);
		--alert-accent: var(--rh-color-warning);
	}

	.alert--danger {
		--alert-bg: var(--rh-color-danger-subtle);
		--alert-border: var(--rh-color-danger-subtle-border);
		--alert-accent: var(--rh-color-danger);
	}

	.alert > :global(.icon) {
		color: var(--alert-accent, var(--rh-color-text-muted));
	}

	.alert__body {
		flex: 1;
		min-width: 0;
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

	.alert__meta {
		display: flex;
		flex-wrap: wrap;
		gap: var(--rh-space-4);
		margin: var(--rh-space-2) 0 0;
		font-size: var(--rh-font-size-sm);
	}

	.alert__meta code {
		padding: 0 var(--rh-space-1);
		border-radius: var(--rh-radius-sm);
		background-color: var(--rh-color-code-bg);
	}
</style>
