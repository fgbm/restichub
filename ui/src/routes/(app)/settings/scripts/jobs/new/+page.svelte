<script lang="ts">
	import { page } from '$app/state';
	import JobTemplateEditor from '#lib/components/JobTemplateEditor.svelte';
</script>

<JobTemplateEditor copyFrom={page.url.searchParams.get('from')} />
