<script lang="ts">
	import { page } from '$app/state';
	import JobTemplateEditor from '#lib/components/JobTemplateEditor.svelte';

	const id = $derived(page.params.id ?? null);
</script>

{#key id}
	<JobTemplateEditor templateId={id} />
{/key}
