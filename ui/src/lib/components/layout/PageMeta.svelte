<script lang="ts">
	import { t } from '#lib/i18n/index.ts';
	import { clearPageMeta, setPageMeta, type Crumb } from '#lib/stores/page-meta.svelte.ts';

	let {
		title,
		crumbs = [],
		listQuery
	}: {
		/** The document title. */
		title: string;
		/** The breadcrumb levels below the section; the last one is this page. */
		crumbs?: Crumb[];
		listQuery?: string;
	} = $props();

	const owner = {};
	$effect(() => {
		setPageMeta({ crumbs, listQuery }, owner);
	});
	$effect(() => () => clearPageMeta(owner));
</script>

<svelte:head>
	<title>{title ? `${title} · ${t('app.name')}` : t('app.name')}</title>
</svelte:head>
