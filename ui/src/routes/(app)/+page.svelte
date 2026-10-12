<script lang="ts">
	import { EmptyState, PageHeader } from '#lib/components/index.ts';
	import { t } from '#lib/i18n/index.ts';
</script>

<PageHeader title={t('overview.title')} description={t('overview.description')} />

<div class="card">
	<EmptyState title={t('overview.title')} description={t('overview.placeholder')} />
</div>

<style>
	.card {
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-surface);
	}
</style>
