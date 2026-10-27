<script lang="ts">
	import { onMount } from 'svelte';
	import { Page } from '#lib/components/layout/index.ts';
	import { Alert, Badge, Button, CopyField, KeyValue, Panel } from '#lib/components/ui/index.ts';
	import type { KeyValueItem } from '#lib/components/ui/index.ts';
	import { t } from '#lib/i18n/index.ts';
	import { getAbout, loadAbout } from '#lib/stores/about.svelte.ts';

	let failed = $state(false);

	async function read(): Promise<void> {
		failed = false;
		await loadAbout();
		failed = getAbout() === null;
	}

	onMount(() => {
		void read();
	});

	const about = $derived(getAbout());
	const failedToLoad = $derived(failed && about === null);

	const facts = $derived<KeyValueItem[]>(
		about
			? [
					{ key: 'version', label: t('about.version'), value: about.version, mono: true },
					{ key: 'revision', label: t('about.revision'), value: about.revision, mono: true },
					{
						key: 'protocol',
						label: t('about.protocol'),
						value: `${about.protocol.major}.${about.protocol.minor}`,
						mono: true
					},
					{
						key: 'agents',
						label: t('about.agents'),
						value: t('about.agents.value', {
							major: about.agent_protocol.major,
							minor: about.agent_protocol.minimum_minor
						}),
						mono: true
					}
				]
			: []
	);

	// The tenancy mode and the database schema level are the Platform Admin's (ADR-0029).
	const installation = $derived<KeyValueItem[]>(
		about?.tenancy_mode && about.schema_level !== null && about.schema_level !== undefined
			? [
					{
						key: 'tenancy',
						label: t('about.tenancyMode'),
						value:
							about.tenancy_mode === 'single' ? t('about.tenancy.single') : t('about.tenancy.multi')
					},
					{
						key: 'schema',
						label: t('about.schemaLevel'),
						value: String(about.schema_level),
						mono: true
					}
				]
			: []
	);

	const support = $derived(
		about
			? t('about.support.text', {
					version: about.version,
					revision: about.revision,
					protocol: `${about.protocol.major}.${about.protocol.minor}`,
					agents: `${about.agent_protocol.major}.${about.agent_protocol.minimum_minor}`
				})
			: ''
	);
</script>

<Page
	title={t('about.title')}
	description={t('about.description')}
	loading={about === null && !failedToLoad}
>
	{#snippet status()}
		{#if about?.prerelease}
			<Badge tone="warning">{t('about.prerelease')}</Badge>
		{/if}
	{/snippet}

	{#if failedToLoad}
		<Alert tone="danger" title={t('about.loadFailed')}>
			{#snippet actions()}
				<Button onclick={() => void read()}>{t('common.retry')}</Button>
			{/snippet}
			<p>{t('about.loadFailedHint')}</p>
		</Alert>
	{:else if about}
		<div class="about">
			<Panel title={t('about.versionPanel')}>
				<KeyValue items={facts} />
			</Panel>

			<Panel title={t('about.support.title')} subtitle={t('about.support.hint')}>
				<CopyField label={t('about.support.label')} value={support} />
			</Panel>

			{#if installation.length > 0}
				<Panel title={t('about.installation.title')}>
					<KeyValue items={installation} />
				</Panel>
			{/if}

			<Panel title={t('about.links.title')}>
				<ul class="about__links">
					<li>
						<a
							href="https://github.com/fgbm/restichub/releases"
							target="_blank"
							rel="noopener noreferrer">{t('about.links.releaseNotes')}</a
						>
					</li>
					<li>
						<a href="https://fgbm.github.io/restichub/" target="_blank" rel="noopener noreferrer"
							>{t('about.links.documentation')}</a
						>
					</li>
					<li>
						<a
							href="https://fgbm.github.io/restichub/reference/agent-version-compatibility/"
							target="_blank"
							rel="noopener noreferrer">{t('about.links.compatibility')}</a
						>
					</li>
					<li>
						<a
							href="https://github.com/fgbm/restichub/blob/main/LICENSE"
							target="_blank"
							rel="noopener noreferrer">{t('about.links.license')}</a
						>
					</li>
				</ul>
			</Panel>
		</div>
	{/if}
</Page>

<style>
	.about {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-4);
		max-width: 720px;
	}

	.about__links {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-2);
		margin: 0;
		padding-left: var(--rh-space-4);
	}
</style>
