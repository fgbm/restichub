<script lang="ts">
	import type { JobDetail } from '#lib/api/index.ts';
	import { Panel } from '#lib/components/ui/index.ts';
	import { t } from '#lib/i18n/index.ts';
	import type { MessageKey } from '#lib/i18n/index.ts';

	let {
		restore,
		hostname
	}: {
		restore: NonNullable<JobDetail['restore']>;
		/** The host that ran the restore. */
		hostname?: string | null;
	} = $props();
</script>

<Panel title={t('jobDetail.restore.title')}>
	<dl class="params">
		<dt>{t('jobDetail.restore.snapshot')}</dt>
		<dd class="mono" title={restore.snapshot_id}>{restore.snapshot_id.slice(0, 8)}</dd>
		<dt>{t('jobDetail.restore.host')}</dt>
		<dd>{hostname ?? t('common.none')}</dd>
		<dt>{t('jobDetail.restore.target')}</dt>
		<dd class="mono">{restore.target}</dd>
		<dt>{t('jobDetail.restore.scope')}</dt>
		<dd>
			{#if restore.includes.length === 0}
				{t('jobDetail.restore.scopeAll')}
			{:else}
				{#each restore.includes as path (path)}
					<span class="mono line">{path}</span>
				{/each}
			{/if}
		</dd>
		{#if restore.excludes.length > 0}
			<dt>{t('jobDetail.restore.excludes')}</dt>
			<dd>
				{#each restore.excludes as path (path)}
					<span class="mono line">{path}</span>
				{/each}
			</dd>
		{/if}
		<dt>{t('jobDetail.restore.overwrite')}</dt>
		<dd>{t(`restore.overwrite.${restore.overwrite}` as MessageKey)}</dd>
		<dt>{t('jobDetail.restore.verify')}</dt>
		<dd>{restore.verify ? t('common.yes') : t('common.no')}</dd>
		<dt>{t('jobDetail.restore.dryRun')}</dt>
		<dd>{restore.dry_run ? t('common.yes') : t('common.no')}</dd>
	</dl>
</Panel>

<style>
	.params {
		display: grid;
		grid-template-columns: minmax(80px, 1fr) fit-content(65%);
		margin: 0;
	}

	.params dt,
	.params dd {
		margin: 0;
		padding: var(--rh-space-2) 0;
		border-top: var(--rh-border-width) solid var(--rh-color-border-muted);
		font-size: var(--rh-font-size-base);
		overflow-wrap: anywhere;
	}

	.params dt {
		padding-inline-end: var(--rh-space-4);
		color: var(--rh-color-text-muted);
	}

	.params dt:first-of-type,
	.params dd:first-of-type {
		border-top: 0;
	}

	.mono {
		font-family: var(--rh-font-mono, monospace);
		font-size: var(--rh-font-size-sm);
	}

	.line {
		display: block;
	}
</style>
