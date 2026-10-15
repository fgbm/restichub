<script lang="ts">
	import type { Snippet } from 'svelte';
	import { AppShell } from '#lib/components/index.ts';
	import { t } from '#lib/i18n/index.ts';
	import { startEvents, stopEvents } from '#lib/stores/events.svelte.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';

	let { children }: { children: Snippet } = $props();

	const session = $derived(getSession());
	// A session in the platform tenant holds no tenant data, so it opens no stream. A switch of the
	// tenant opens a new stream for the new tenant.
	const streamTenant = $derived(session?.tenant?.id ?? null);

	$effect(() => {
		if (!streamTenant) return;
		startEvents();
		return stopEvents;
	});
</script>

{#if session}
	<AppShell>{@render children()}</AppShell>
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
