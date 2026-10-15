<script lang="ts">
	import { parseRich } from '#lib/rich-text.ts';

	let { text }: { text: string } = $props();

	const segments = $derived(parseRich(text));
</script>

{#each segments as segment, index (index)}{#if segment.tag === 'b'}<b>{segment.text}</b
		>{:else if segment.tag === 'code'}<code>{segment.text}</code>{:else}{segment.text}{/if}{/each}
