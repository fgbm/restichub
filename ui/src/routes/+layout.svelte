<script lang="ts">
	import type { Snippet } from 'svelte';
	import { onMount } from 'svelte';
	import '#lib/styles/tokens.css';
	import { applyLocale } from '#lib/i18n/index.ts';
	import { applyTheme } from '#lib/stores/theme.svelte.ts';

	let { children }: { children: Snippet } = $props();

	onMount(() => {
		applyTheme();
		applyLocale();
	});
</script>

{@render children()}
