<script lang="ts">
	import { Alert, Card, EmptyState, Icon, PageHeader, Panel } from '#lib/components/ui/index.ts';
	import { t } from '#lib/i18n/index.ts';

	// The onboarding steps of the first run. Step one is done: the person is signed in,
	// so the installation has its first administrator.
	const steps = [
		{ title: t('overview.step.admin.title'), body: t('overview.step.admin.body') },
		{ title: t('overview.step.agent.title'), body: t('overview.step.agent.body') },
		{ title: t('overview.step.repository.title'), body: t('overview.step.repository.body') },
		{ title: t('overview.step.profile.title'), body: t('overview.step.profile.body') }
	];
</script>

<svelte:head>
	<title>{t('overview.title')} · {t('app.name')}</title>
</svelte:head>

<PageHeader title={t('overview.title')} description={t('overview.description')} bordered />

<div class="banner">
	<Alert tone="info" title={t('overview.welcome.title')}>{t('overview.welcome.body')}</Alert>
</div>

<Panel title={t('overview.getStarted')}>
	<ol class="steps">
		{#each steps as step, index (step.title)}
			<li class="step" class:step--done={index === 0}>
				<span class="step__mark">
					{#if index === 0}
						<Icon name="i-check" size="xs" />
					{:else}
						{index + 1}
					{/if}
				</span>
				<div class="step__main">
					<p class="step__title">{step.title}</p>
					<p class="step__body">{step.body}</p>
				</div>
			</li>
		{/each}
	</ol>
</Panel>

<div class="grid">
	<Card padded={false}>
		<EmptyState
			icon="i-host"
			title={t('overview.empty.hosts.title')}
			description={t('overview.empty.hosts.body')}
		/>
	</Card>
	<Card padded={false}>
		<EmptyState
			icon="i-job"
			title={t('overview.empty.jobs.title')}
			description={t('overview.empty.jobs.body')}
		/>
	</Card>
	<Card padded={false}>
		<EmptyState
			icon="i-repository"
			title={t('overview.empty.repositories.title')}
			description={t('overview.empty.repositories.body')}
		/>
	</Card>
</div>

<Panel title={t('overview.how.title')}>
	<ul class="hints">
		<li><Icon name="i-check" size="xs" /> {t('overview.how.control')}</li>
		<li><Icon name="i-check" size="xs" /> {t('overview.how.agent')}</li>
		<li><Icon name="i-check" size="xs" /> {t('overview.how.maintenance')}</li>
	</ul>
</Panel>

<style>
	.banner {
		margin-bottom: var(--rh-space-6);
	}

	.steps {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-4);
		margin: 0;
		padding: 0;
		list-style: none;
	}

	.step {
		display: flex;
		align-items: flex-start;
		gap: var(--rh-space-3);
	}

	.step__mark {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		flex: none;
		width: 24px;
		height: 24px;
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-pill);
		background-color: var(--rh-color-canvas-subtle);
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-xs);
		font-weight: var(--rh-font-weight-semibold);
	}

	.step--done .step__mark {
		border-color: var(--rh-color-success-subtle-border);
		background-color: var(--rh-color-success-subtle);
		color: var(--rh-color-success-subtle-text);
	}

	.step__main {
		min-width: 0;
	}

	.step__title {
		margin: 0;
		font-weight: var(--rh-font-weight-semibold);
	}

	.step__body {
		margin: var(--rh-space-1) 0 0;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.grid {
		display: grid;
		grid-template-columns: repeat(auto-fit, minmax(18rem, 1fr));
		gap: var(--rh-space-4);
		margin: var(--rh-space-6) 0;
	}

	.hints {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-3);
		margin: 0;
		padding: 0;
		list-style: none;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.hints li {
		display: flex;
		align-items: flex-start;
		gap: var(--rh-space-2);
	}

	.hints :global(.icon) {
		margin-top: 2px;
		color: var(--rh-color-success);
	}
</style>
