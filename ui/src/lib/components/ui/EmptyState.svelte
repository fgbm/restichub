<script lang="ts">
	import type { Snippet } from 'svelte';
	import Icon from './Icon.svelte';
	import type { IconName } from './icons';

	let {
		title,
		description,
		icon,
		bordered = false,
		children
	}: {
		title: string;
		description?: string;
		icon?: IconName;
		bordered?: boolean;
		children?: Snippet;
	} = $props();
</script>

<div class="empty" class:empty--bordered={bordered}>
	{#if icon}
		<span class="empty__icon"><Icon name={icon} size="xl" /></span>
	{/if}
	<p class="empty__title">{title}</p>
	{#if description}
		<p class="empty__body">{description}</p>
	{/if}
	{#if children}
		<div class="empty__actions">{@render children()}</div>
	{/if}
</div>

<style>
	.empty {
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: var(--rh-space-3);
		padding: var(--rh-space-10) var(--rh-space-4);
		color: var(--rh-color-text-muted);
		text-align: center;
	}

	.empty--bordered {
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
	}

	.empty__icon {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		width: 48px;
		height: 48px;
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-pill);
		background-color: var(--rh-color-canvas-subtle);
		color: var(--rh-color-text-subtle);
	}

	.empty__title {
		margin: 0;
		font-size: var(--rh-font-size-lg);
		font-weight: var(--rh-font-weight-semibold);
		color: var(--rh-color-text);
	}

	.empty__body {
		margin: 0;
		max-width: 44ch;
		font-size: var(--rh-font-size-sm);
	}

	.empty__actions {
		display: flex;
		gap: var(--rh-space-2);
		margin-top: var(--rh-space-1);
	}
</style>
