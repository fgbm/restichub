<script lang="ts">
	import { page } from '$app/state';
	import SnapshotPage from '#lib/components/SnapshotPage.svelte';

	// A new snapshot in the same route starts with a fresh page: the tree, the selection and the
	// loaded records of the previous snapshot belong to that snapshot only.
	const repositoryId = $derived(page.params.repository ?? '');
	const snapshotId = $derived(page.params.snapshot ?? '');
</script>

{#key `${repositoryId}/${snapshotId}`}
	<SnapshotPage {repositoryId} {snapshotId} />
{/key}
