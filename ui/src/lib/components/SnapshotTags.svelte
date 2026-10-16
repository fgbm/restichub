<script lang="ts">
	import { Badge } from '#lib/components/ui/index.ts';
	import { t } from '#lib/i18n/index.ts';
	import { profileOfTag } from '#lib/restore.ts';

	let {
		tags,
		profiles = new Map<string, string>()
	}: {
		tags: string[];
		/** The names of the profiles by identifier, for the automatic profile tags. */
		profiles?: ReadonlyMap<string, string>;
	} = $props();

	function label(tag: string): string {
		const profile = profileOfTag(tag);
		if (!profile) return tag;
		return t('restore.profileTag', { name: profiles.get(profile) ?? profile.slice(0, 8) });
	}
</script>

<span class="tags">
	{#each tags as tag (tag)}
		<span class="tags__chip" title={tag}><Badge>{label(tag)}</Badge></span>
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
