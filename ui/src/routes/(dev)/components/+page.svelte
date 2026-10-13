<script lang="ts">
	import ComponentGallery from '#lib/screens/ComponentGallery.svelte';
	import { t } from '#lib/i18n/index.ts';
</script>

<svelte:head>
	<title>Components · {t('app.name')}</title>
</svelte:head>

<ComponentGallery />
