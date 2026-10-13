<script lang="ts">
	import type { Snippet } from 'svelte';
	import { AppShell } from '#lib/components/index.ts';
	import { t } from '#lib/i18n/index.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';

	let { children }: { children: Snippet } = $props();

	const session = $derived(getSession());
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
