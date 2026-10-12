<script lang="ts">
	import type { Snippet } from 'svelte';
	import { t } from '#lib/i18n/index.ts';

	type Tone = 'info' | 'success' | 'warning' | 'danger';

	let {
		tone = 'danger',
		title,
		code,
		requestId,
		children
	}: {
		tone?: Tone;
		title: string;
		code?: string;
		requestId?: string;
		children?: Snippet;
	} = $props();
</script>

<div class="alert alert--{tone}" role={tone === 'danger' ? 'alert' : 'status'}>
	<div class="alert__content">
		<p class="alert__title">{title}</p>
		{#if children}
			<div class="alert__body">{@render children()}</div>
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
		padding: var(--rh-space-3) var(--rh-space-4);
		border: var(--rh-border-width) solid transparent;
		border-radius: var(--rh-radius-sm);
		font-size: var(--rh-font-size-base);
	}

	.alert--info {
		background-color: var(--rh-color-info-subtle);
		border-color: var(--rh-color-info);
		color: var(--rh-color-info-subtle-text);
	}

	.alert--success {
		background-color: var(--rh-color-success-subtle);
		border-color: var(--rh-color-success);
		color: var(--rh-color-success-subtle-text);
	}

	.alert--warning {
		background-color: var(--rh-color-warning-subtle);
		border-color: var(--rh-color-warning);
		color: var(--rh-color-warning-subtle-text);
	}

	.alert--danger {
		background-color: var(--rh-color-danger-subtle);
		border-color: var(--rh-color-danger);
		color: var(--rh-color-danger-subtle-text);
	}

	.alert__title {
		margin: 0;
		font-weight: var(--rh-font-weight-semibold);
	}

	.alert__body {
		margin-top: var(--rh-space-1);
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
		background-color: var(--rh-color-surface);
	}
</style>
