<script lang="ts">
	import { page } from '$app/state';
	import ScriptTemplateEditor from '#lib/components/ScriptTemplateEditor.svelte';
</script>

<ScriptTemplateEditor copyFrom={page.url.searchParams.get('from')} />
