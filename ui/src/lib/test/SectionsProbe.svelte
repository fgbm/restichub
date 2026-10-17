<script lang="ts">
	import { rememberSections } from '#lib/section-state.svelte.ts';

	let { kind = 'probe' }: { kind?: string } = $props();

	const SECTIONS = ['basics', 'paths', 'tags'];
	// svelte-ignore state_referenced_locally
	const open = rememberSections(kind, SECTIONS, { basics: true, paths: true, tags: false });
</script>

{#each SECTIONS as id (id)}
	<label><input type="checkbox" bind:checked={open[id]} />{id}</label>
{/each}
