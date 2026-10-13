<script lang="ts">
	import type { Snippet } from 'svelte';
	import { Button } from '#lib/components/ui/index.ts';
	import { t } from '#lib/i18n/index.ts';
	import { getTheme, setTheme } from '#lib/stores/theme.svelte.ts';

	let { children }: { children: Snippet } = $props();

	let prefersDark = $state(
		typeof window !== 'undefined' && window.matchMedia('(prefers-color-scheme: dark)').matches
	);
	const dark = $derived(getTheme() === 'dark' || (getTheme() === 'system' && prefersDark));

	$effect(() => {
		const media = window.matchMedia('(prefers-color-scheme: dark)');
		const update = (): void => {
			prefersDark = media.matches;
		};
		media.addEventListener('change', update);
		return () => media.removeEventListener('change', update);
	});
</script>

<div class="auth-topbar">
	<Button
		icon={dark ? 'sun' : 'moon'}
		aria-label={dark ? t('theme.switchToLight') : t('theme.switchToDark')}
		title={dark ? t('theme.switchToLight') : t('theme.switchToDark')}
		onclick={() => setTheme(dark ? 'light' : 'dark')}
	/>
</div>

<main class="auth-shell">
	{@render children()}
</main>

<style>
	.auth-topbar {
		position: fixed;
		top: var(--rh-space-4);
		right: var(--rh-space-4);
		display: flex;
		align-items: center;
		gap: var(--rh-space-2);
	}

	.auth-shell {
		display: flex;
		flex-direction: column;
		align-items: center;
		justify-content: center;
		min-height: 100vh;
		padding: var(--rh-space-6);
		background-color: var(--rh-color-canvas);
	}
</style>
