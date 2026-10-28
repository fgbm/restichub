<script lang="ts">
	import type { Snippet } from 'svelte';
	import { onMount } from 'svelte';
	import '#lib/styles/tokens.css';
	import { applyLocale } from '#lib/i18n/index.ts';
	import { applyTheme } from '#lib/stores/theme.svelte.ts';
	import { watchWindowScrollbar } from '#lib/scrollbar.ts';

	let { children }: { children: Snippet } = $props();

	onMount(() => {
		applyTheme();
		applyLocale();
		return watchWindowScrollbar();
	});
</script>

{@render children()}
