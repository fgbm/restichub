<script lang="ts">
	import { Alert, Button, EmptyState, Icon, PageHeader, Panel } from '#lib/components/ui/index.ts';
	import type { IconName } from '#lib/components/ui/index.ts';
	import { t } from '#lib/i18n/index.ts';
	import type { MessageKey } from '#lib/i18n/index.ts';
	import { isSingleTenant } from '#lib/stores/instance.svelte.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';

	interface Step {
		title: string;
		body: string;
		done: boolean;
		action?: string;
	}

	const session = $derived(getSession());

	// Only the first step can be done here: the hosts, repositories and profiles
	// screens arrive later, so their actions stay disabled.
	const steps = $derived<Step[]>([
		{
			title: isSingleTenant() ? t('setup.submitSingle') : t('overview.step.admin.title'),
			body: t('overview.step.admin.body', {
				name: session?.user?.display_name || session?.user?.username || ''
			}),
			done: true
		},
		{
			title: t('overview.step.agent.title'),
			body: t('overview.step.agent.body'),
			done: false,
			action: t('overview.step.agent.action')
		},
		{
			title: t('overview.step.repository.title'),
			body: t('overview.step.repository.body'),
			done: false,
			action: t('overview.step.repository.action')
		},
		{
			title: t('overview.step.profile.title'),
			body: t('overview.step.profile.body'),
			done: false,
			action: t('overview.step.profile.action')
		}
	]);

	const done = $derived(steps.filter((step) => step.done).length);

	const empties: { icon: IconName; title: MessageKey; body: MessageKey }[] = [
		{ icon: 'host', title: 'overview.empty.hosts.title', body: 'overview.empty.hosts.body' },
		{ icon: 'job', title: 'overview.empty.jobs.title', body: 'overview.empty.jobs.body' },
		{
			icon: 'repository',
			title: 'overview.empty.repositories.title',
			body: 'overview.empty.repositories.body'
		}
	];

	const hints: MessageKey[] = [
		'overview.how.schedule',
		'overview.how.data',
		'overview.how.maintenance'
	];
</script>

<svelte:head>
	<title>{t('overview.title')} · {t('app.name')}</title>
</svelte:head>

<PageHeader title={t('overview.title')} description={t('overview.description')} bordered />

<div class="overview">
	<Alert tone="info" title={t('overview.welcomeTitle')}>{t('overview.welcomeBody')}</Alert>

	<Panel title={t('overview.getStarted')} flush>
		{#snippet actions()}
			<span>{t('overview.progress', { done, total: steps.length })}</span>
		{/snippet}
		<ol class="checklist">
			{#each steps as step, index (step.title)}
				<li class="checklist__item" class:checklist__item--done={step.done}>
					<span class="checklist__mark">
						{#if step.done}
							<Icon name="check" size="xs" />
						{:else}
							{index + 1}
						{/if}
					</span>
					<div class="checklist__main">
						<p class="checklist__title">{step.title}</p>
						<p class="checklist__body">{step.body}</p>
					</div>
					{#if step.action}
						<Button disabled>{step.action}</Button>
					{/if}
				</li>
			{/each}
		</ol>
	</Panel>

	<div class="empties">
		{#each empties as empty (empty.icon)}
			<EmptyState icon={empty.icon} title={t(empty.title)} description={t(empty.body)} bordered />
		{/each}
	</div>

	<Panel title={t('overview.how.title')}>
		<ul class="hints">
			{#each hints as hint (hint)}
				<li><Icon name="check" size="xs" />{t(hint)}</li>
			{/each}
		</ul>
	</Panel>
</div>

<style>
	.overview {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-6);
	}

	.checklist {
		margin: 0;
		padding: 0;
		list-style: none;
	}

	.checklist__item {
		display: flex;
		align-items: flex-start;
		gap: var(--rh-space-3);
		padding: var(--rh-space-4);
		border-top: var(--rh-border-width) solid var(--rh-color-border-muted);
	}

	.checklist__item:first-child {
		border-top: 0;
	}

	.checklist__item--done {
		color: var(--rh-color-text-muted);
	}

	.checklist__mark {
		display: inline-flex;
		flex: none;
		align-items: center;
		justify-content: center;
		width: 22px;
		height: 22px;
		margin-top: 1px;
		border: var(--rh-border-width) solid var(--rh-color-border-strong);
		border-radius: var(--rh-radius-pill);
		color: var(--rh-color-text-subtle);
		font-size: var(--rh-font-size-sm);
	}

	.checklist__item--done .checklist__mark {
		border-color: var(--rh-color-success-subtle-border);
		background-color: var(--rh-color-success-subtle);
		color: var(--rh-color-success-text);
	}

	.checklist__main {
		flex: 1 1 auto;
		min-width: 0;
	}

	.checklist__title {
		margin: 0;
		font-weight: var(--rh-font-weight-semibold);
	}

	.checklist__body {
		margin: 2px 0 0;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.empties {
		display: grid;
		grid-template-columns: repeat(3, minmax(0, 1fr));
		gap: var(--rh-space-4);
	}

	@media (max-width: 900px) {
		.empties {
			grid-template-columns: minmax(0, 1fr);
		}
	}

	.hints {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-2);
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
		color: var(--rh-color-text-subtle);
	}
</style>
