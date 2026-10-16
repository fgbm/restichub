<script lang="ts">
	import { page } from '$app/state';
	import ProfileEditor from '#lib/components/ProfileEditor.svelte';
</script>

<ProfileEditor copyFrom={page.url.searchParams.get('from')} />
