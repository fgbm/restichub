<script lang="ts">
	import { Card, EmptyState, PageHeader } from '#lib/components/ui/index.ts';
	import { t } from '#lib/i18n/index.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';

	// An account with no current organization sees the empty state (ADR-0029).
	const hasTenant = $derived(!!getSession()?.tenant);
</script>

<svelte:head>
	<title>{t('overview.title')} · {t('app.name')}</title>
</svelte:head>

<PageHeader title={t('overview.title')} description={t('overview.description')} />

{#if hasTenant}
	<Card>
		<EmptyState title={t('overview.title')} description={t('overview.placeholder')} />
	</Card>
{:else}
	<Card>
		<EmptyState title={t('overview.title')} description={t('overview.noOrganizations')} />
	</Card>
{/if}
