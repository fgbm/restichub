<script lang="ts">
	import { Badge } from '#lib/components/ui/index.ts';
	import { tagLabel } from '#lib/snapshot-filter.ts';

	let {
		tags,
		profiles = new Map<string, string>()
	}: {
		tags: string[];
		/** The names of the profiles by identifier, for the automatic profile tags. */
		profiles?: ReadonlyMap<string, string>;
	} = $props();
</script>

<span class="tags">
	{#each tags as tag (tag)}
		<span class="tags__chip" title={tag}><Badge>{tagLabel(tag, profiles)}</Badge></span>
	{/each}
</span>

<style>
	.tags {
		display: inline-flex;
		flex-wrap: wrap;
		gap: var(--rh-space-1);
		min-width: 0;
	}

	.tags__chip {
		position: relative;
		z-index: 1;
		max-width: 100%;
	}
</style>
