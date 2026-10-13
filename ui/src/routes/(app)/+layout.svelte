<script lang="ts">
	import type { Snippet } from 'svelte';
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { AppShell } from '#lib/components/index.ts';
	import { t } from '#lib/i18n/index.ts';
	import { landingHref } from '#lib/navigation.ts';
	import { isInstanceLoaded, isSingleTenant, loadInstance } from '#lib/stores/instance.svelte.ts';
	import { getSession, isLoaded, loadSession } from '#lib/stores/session.svelte.ts';

	let { children }: { children: Snippet } = $props();

	const session = $derived(getSession());
	const loaded = $derived(isLoaded());

	onMount(async () => {
		if (!isInstanceLoaded()) await loadInstance();
		if (!isLoaded()) await loadSession();
		if (!getSession()) await goto(resolve('/(auth)/sign-in'));
	});

	$effect(() => {
		if (!session || page.url.pathname !== resolve('/(app)')) return;
		const target = landingHref(session, isSingleTenant());
		if (target !== '/(app)') void goto(resolve(target), { replaceState: true });
	});
</script>

{#if session}
	<AppShell>{@render children()}</AppShell>
{:else if loaded}
	<main class="centered">
		<p>{t('common.loading')}</p>
	</main>
{:else}
	<main class="centered">
		<p>{t('common.loading')}</p>
	</main>
{/if}

<style>
	.centered {
		display: flex;
		align-items: center;
		justify-content: center;
		min-height: 100vh;
		color: var(--rh-color-text-muted);
	}
</style>
