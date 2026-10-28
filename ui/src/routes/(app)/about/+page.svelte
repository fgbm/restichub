<script lang="ts">
	import { onMount } from 'svelte';
	import type { About, AboutEnvironment } from '#lib/api/index.ts';
	import { Page } from '#lib/components/layout/index.ts';
	import { Alert, Badge, Button, CopyField, KeyValue, Panel } from '#lib/components/ui/index.ts';
	import type { KeyValueItem } from '#lib/components/ui/index.ts';
	import { getLocale, t } from '#lib/i18n/index.ts';
	import { getAbout, isAboutFailed, loadAbout, retryAbout } from '#lib/stores/about.svelte.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';
	import { getTenancyMode } from '#lib/stores/instance.svelte.ts';

	onMount(() => {
		void loadAbout();
	});

	const about = $derived(getAbout());
	const failed = $derived(isAboutFailed() && about === null);
	const session = $derived(getSession());
	const locale = $derived(getLocale());

	// The tenancy mode is public on `/api/v1/instance`; the schema level is the Platform Admin's
	// (ADR-0054).
	const platformAdmin = $derived(session?.platform_admin ?? false);
	const docsRoot = $derived(
		locale === 'ru' ? 'https://fgbm.github.io/restichub/ru/' : 'https://fgbm.github.io/restichub/'
	);

	const agents = $derived(
		about
			? t('about.agents.value', {
					major: about.agent_protocol.major,
					minor: about.agent_protocol.minimum_minor
				})
			: ''
	);

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
					{ key: 'agents', label: t('about.agents'), value: agents, mono: true }
				]
			: []
	);

	const installation = $derived<KeyValueItem[]>(
		about && platformAdmin
			? [
					{
						key: 'tenancy',
						label: t('about.tenancyMode'),
						value:
							getTenancyMode() === 'single' ? t('about.tenancy.single') : t('about.tenancy.multi')
					},
					{
						key: 'schema',
						label: t('about.schemaLevel'),
						value:
							about.schema_level === null || about.schema_level === undefined
								? t('common.notSet')
								: String(about.schema_level),
						mono: about.schema_level !== null && about.schema_level !== undefined
					}
				]
			: []
	);

	// The tenancy mode is public on `/api/v1/instance`, so the support text holds it for every role.
	const tenancy = $derived(
		getTenancyMode() === 'single'
			? t('about.support.tenancy.single')
			: t('about.support.tenancy.multi')
	);

	const environment = $derived<KeyValueItem[]>(
		about
			? [
					fact('os', t('about.environment.os'), about.environment.os),
					fact('osVersion', t('about.environment.osVersion'), about.environment.os_version, true),
					fact('kernel', t('about.environment.kernel'), about.environment.kernel, true),
					fact('arch', t('about.environment.arch'), about.environment.arch, true),
					fact('container', t('about.environment.container'), containerState(about.environment)),
					fact('database', t('about.environment.database'), about.environment.database, true)
				]
			: []
	);

	const support = $derived(about ? supportText(about, agents, tenancy) : '');

	// A locked fact shows its value, or "Not set" while the process has no answer (ui-principles 3.9).
	function fact(
		key: string,
		label: string,
		value: string | null | undefined,
		mono = false
	): KeyValueItem {
		return value === null || value === undefined
			? { key, label, value: t('common.notSet') }
			: { key, label, value, mono };
	}

	function systemName(environment: AboutEnvironment): string {
		return environment.os_version
			? t('about.environment.osVersioned', {
					distribution: environment.os_version,
					os: environment.os
				})
			: environment.os;
	}

	function containerState(environment: AboutEnvironment): string | null {
		if (environment.container === null || environment.container === undefined) {
			return null;
		}
		return environment.container ? t('common.yes') : t('common.no');
	}

	function containerName(environment: AboutEnvironment): string {
		if (environment.container === null || environment.container === undefined) {
			return t('about.support.container.notSet');
		}
		return environment.container
			? t('about.support.container.yes')
			: t('about.support.container.no');
	}

	// One copy of the text is enough for support, so every line holds the facts of one area.
	function supportText(value: About, agentRange: string, mode: string): string {
		const name = systemName(value.environment);
		const container = containerName(value.environment);
		const system = value.environment.kernel
			? t('about.support.system', {
					os: name,
					kernel: value.environment.kernel,
					arch: value.environment.arch,
					container
				})
			: t('about.support.systemNoKernel', {
					os: name,
					arch: value.environment.arch,
					container
				});
		return [
			t('about.support.version', { version: value.version, revision: value.revision }),
			t('about.support.protocol', {
				protocol: `${value.protocol.major}.${value.protocol.minor}`,
				agents: agentRange
			}),
			system,
			t('about.support.database', {
				database: value.environment.database ?? t('common.notSet')
			}),
			t('about.support.tenancy', { tenancy: mode })
		].join('\n');
	}
</script>

<Page
	title={t('about.title')}
	description={t('about.description')}
	loading={about === null && !failed}
>
	{#snippet status()}
		{#if about?.prerelease}
			<Badge tone="warning">{t('about.prerelease')}</Badge>
		{/if}
	{/snippet}

	{#if failed}
		<Alert tone="danger" title={t('about.loadFailed')}>
			{#snippet actions()}
				<Button onclick={() => void retryAbout()}>{t('common.retry')}</Button>
			{/snippet}
			<p>{t('about.loadFailedHint')}</p>
		</Alert>
	{:else if about}
		<div class="about">
			<Panel title={t('about.versionPanel')}>
				<KeyValue items={facts} />
			</Panel>

			<Panel title={t('about.environment.title')}>
				<KeyValue items={environment} />
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
				<!-- The documentation site is outside the application, so the addresses are no routes. -->
				<!-- eslint-disable svelte/no-navigation-without-resolve -->
				<ul class="about__links">
					<li>
						<a
							href="https://github.com/fgbm/restichub/releases"
							target="_blank"
							rel="noopener noreferrer">{t('about.links.releaseNotes')}</a
						>
					</li>
					<li>
						<a href={docsRoot} target="_blank" rel="noopener noreferrer"
							>{t('about.links.documentation')}</a
						>
					</li>
					<li>
						<a
							href={`${docsRoot}reference/agent-version-compatibility/`}
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
				<!-- eslint-enable svelte/no-navigation-without-resolve -->
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
