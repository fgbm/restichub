<script lang="ts">
	import { page } from '$app/state';
	import ProfileEditor from '#lib/components/ProfileEditor.svelte';

	const id = $derived(page.params.id ?? null);
</script>

{#key id}
	<ProfileEditor profileId={id} />
{/key}
