<script lang="ts">
	import { page } from '$app/state';
	import OrgUserPage from '#lib/components/OrgUserPage.svelte';

	const id = $derived(page.params.id ?? '');
</script>

{#key id}
	<OrgUserPage userId={id} />
{/key}
