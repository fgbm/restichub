<script lang="ts">
	import type { Snippet } from 'svelte';
	import { Icon, Switch } from './ui/index.ts';
	import type { SummaryState } from '#lib/profile-form.ts';
	import { t } from '#lib/i18n/index.ts';

	let {
		id,
		title,
		summary,
		state,
		open = $bindable(false),
		enabled = $bindable(true),
		switchable = false,
		required = false,
		badge,
		children
	}: {
		id: string;
		title: string;
		summary: string;
		state: SummaryState;
		open?: boolean;
		enabled?: boolean;
		/** The header holds an on/off switch. A switched-off section keeps its fields dimmed. */
		switchable?: boolean;
		required?: boolean;
		badge?: string;
		children: Snippet;
	} = $props();

	const bodyId = $derived(`sec-${id}`);
	const stateLabel = $derived(
		state === 'warn' ? t('pe.state.warn') : state === 'on' ? t('pe.state.on') : t('pe.state.off')
	);

	function toggle(): void {
		open = !open;
	}

	// Switching a section on opens it, so the operator sees what the switch turned on.
	let wasEnabled = enabled;
	$effect(() => {
		if (enabled && !wasEnabled) open = true;
		wasEnabled = enabled;
	});
</script>

<section class="section" class:section--open={open} data-section={id}>
	<header class="section__head">
		<button
			type="button"
			class="section__toggle"
			aria-expanded={open}
			aria-controls={bodyId}
			onclick={toggle}
		>
			<Icon name={open ? 'chevron-down' : 'chevron-right'} size="xs" />
			<span class="dot dot--{state}" title={stateLabel} aria-hidden="true" data-state={state}
			></span>
			<span class="section__title">
				{title}{#if required}<span class="section__req" aria-hidden="true">*</span>{/if}
			</span>
			{#if badge}<span class="section__badge">{badge}</span>{/if}
			{#if !open}<span class="section__summary">{summary}</span>{/if}
		</button>
		{#if switchable}
			<Switch bind:checked={enabled} label={t('pe.section.switch', { title })} />
		{/if}
	</header>
	{#if open}
		<div class="section__body" class:section__body--off={switchable && !enabled} id={bodyId}>
			{@render children()}
		</div>
	{/if}
</section>

<style>
	.section {
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-lg);
		background-color: var(--rh-color-surface);
	}

	.section__head {
		display: flex;
		align-items: center;
		gap: var(--rh-space-3);
		padding-right: var(--rh-space-3);
	}

	.section__toggle {
		display: flex;
		flex: 1;
		align-items: center;
		gap: var(--rh-space-2);
		min-width: 0;
		padding: var(--rh-space-3) var(--rh-space-3);
		border: 0;
		background: none;
		color: var(--rh-color-text);
		font: inherit;
		text-align: left;
		cursor: pointer;
	}

	.section__title {
		font-weight: var(--rh-font-weight-semibold);
		white-space: nowrap;
	}

	.section__req {
		margin-left: 2px;
		color: var(--rh-color-danger-text);
	}

	.section__badge {
		padding: 0 var(--rh-space-2);
		border-radius: var(--rh-radius-pill);
		background-color: var(--rh-color-info-subtle);
		color: var(--rh-color-info-subtle-text);
		font-size: var(--rh-font-size-xs);
	}

	.section__summary {
		overflow: hidden;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.section__head :global(.rh-switch__label) {
		display: none;
	}

	.section__body {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-4);
		padding: var(--rh-space-1) var(--rh-space-4) var(--rh-space-4);
	}

	.section__body--off {
		opacity: 0.55;
	}

	.dot {
		flex: none;
		width: 8px;
		height: 8px;
		border-radius: 50%;
		background-color: var(--rh-color-border-strong);
	}

	.dot--on {
		background-color: var(--rh-color-success);
	}

	.dot--warn {
		background-color: var(--rh-color-warning);
	}
</style>
