<script lang="ts">
	import { page } from '$app/state';
	import ScriptTemplateEditor from '#lib/components/ScriptTemplateEditor.svelte';

	const id = $derived(page.params.id ?? null);
</script>

{#key id}
	<ScriptTemplateEditor templateId={id} />
{/key}
