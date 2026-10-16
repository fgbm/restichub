<script lang="ts">
	import { page } from '$app/state';
	import RepositoryEditor from '#lib/components/RepositoryEditor.svelte';

	const id = $derived(page.params.id ?? null);
</script>

{#key id}
	<RepositoryEditor repositoryId={id} />
{/key}
