<script lang="ts">
	import type { Snippet } from 'svelte';
	import Brandmark from './Brandmark.svelte';

	let {
		title,
		description,
		wide = false,
		children,
		foot
	}: {
		title: string;
		description?: string;
		/** The setup form is wider than the sign-in form. */
		wide?: boolean;
		children: Snippet;
		foot?: Snippet;
	} = $props();
</script>

<div class="auth-card" class:auth-card--wide={wide}>
	<div class="auth-head">
		<Brandmark size="lg" />
		<div>
			<h1 class="auth-title">{title}</h1>
			{#if description}
				<p class="auth-description">{description}</p>
			{/if}
		</div>
	</div>
	<div class="form-box">{@render children()}</div>
	{#if foot}
		<p class="auth-foot">{@render foot()}</p>
	{/if}
</div>

<style>
	.auth-card {
		width: 100%;
		max-width: 400px;
	}

	.auth-card--wide {
		max-width: 440px;
	}

	.auth-head {
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: var(--rh-space-3);
		margin-bottom: var(--rh-space-5);
		text-align: center;
	}

	.auth-title {
		margin: 0;
		font-size: var(--rh-font-size-xl);
		font-weight: var(--rh-font-weight-semibold);
		line-height: var(--rh-line-height-tight);
	}

	.auth-description {
		margin: 0;
		color: var(--rh-color-text-muted);
	}

	.form-box {
		padding: var(--rh-space-5);
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-lg);
		background-color: var(--rh-color-canvas);
	}

	.auth-foot {
		margin: var(--rh-space-4) 0 0;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
		text-align: center;
	}
</style>
