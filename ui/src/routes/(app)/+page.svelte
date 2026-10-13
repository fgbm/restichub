<script lang="ts">
	import { Card, EmptyState, PageHeader } from '#lib/components/ui/index.ts';
	import { t } from '#lib/i18n/index.ts';
</script>

<PageHeader title={t('overview.title')} description={t('overview.description')} />

<Card>
	<EmptyState title={t('overview.title')} description={t('overview.placeholder')} />
</Card>
