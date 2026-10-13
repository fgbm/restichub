<script lang="ts">
	import type { Snippet } from 'svelte';
	import { Button } from '#lib/components/ui/index.ts';
	import { t } from '#lib/i18n/index.ts';
	import { getTheme, setTheme } from '#lib/stores/theme.svelte.ts';

	let { children }: { children: Snippet } = $props();

	const theme = $derived(getTheme());
	const dark = $derived(theme === 'dark');
</script>

<main class="auth">
	<div class="auth__topbar">
		<Button
			variant="ghost"
			icon={dark ? 'i-sun' : 'i-moon'}
			aria-label={t('topbar.theme')}
			onclick={() => setTheme(dark ? 'light' : 'dark')}
		/>
	</div>
	<div class="auth__card">
		<span class="auth__mark" aria-hidden="true">
			<svg viewBox="0 0 20 20">
				<path class="auth__mark-solid" d="M10 2.5 2.5 6.4 10 10.3l7.5-3.9z" />
				<path d="M2.5 10.6 10 14.5l7.5-3.9" />
			</svg>
		</span>
		{@render children()}
	</div>
</main>

<style>
	.auth {
		display: flex;
		flex-direction: column;
		align-items: center;
		justify-content: center;
		min-height: 100vh;
		padding: var(--rh-space-6) var(--rh-space-4);
	}

	.auth__topbar {
		position: fixed;
		top: var(--rh-space-3);
		right: var(--rh-space-3);
	}

	.auth__card {
		width: 100%;
		max-width: 26rem;
		padding: var(--rh-space-6);
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-lg);
		background-color: var(--rh-color-surface);
	}

	.auth__mark {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		width: 44px;
		height: 44px;
		margin-bottom: var(--rh-space-4);
		border-radius: 11px;
		background-color: var(--rh-color-accent);
		color: var(--rh-color-text-on-accent);
	}

	.auth__mark svg {
		width: 26px;
		height: 26px;
		fill: none;
		stroke: currentColor;
		stroke-width: 1.7;
		stroke-linejoin: round;
		stroke-linecap: round;
	}

	.auth__mark-solid {
		fill: currentColor;
		stroke: none;
	}
</style>
