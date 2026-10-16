<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { ApiError, service } from '#lib/api/index.ts';
	import type { Agent, ApiProblem, PreviewResponse, Profile } from '#lib/api/index.ts';
	import {
		Alert,
		Badge,
		Button,
		ConfirmDialog,
		EmptyState,
		PageHeader,
		Select,
		SidePanel,
		Table,
		RowActions
	} from '#lib/components/ui/index.ts';
	import type { MenuItem, SelectOption, TableColumn } from '#lib/components/ui/index.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';

	let profiles = $state<Profile[]>([]);
	let repositories = $state<{ id: string; name: string }[]>([]);
	let agents = $state<Agent[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);

	let deleteTarget = $state<Profile | null>(null);
	let deleteOpen = $state(false);
	let deleting = $state(false);

	let previewOpen = $state(false);
	let previewTarget = $state<Profile | null>(null);
	let previewAgentId = $state('');
	let previewLoading = $state(false);
	let previewResult = $state<PreviewResponse | null>(null);
	let previewProblem = $state<ApiProblem | null>(null);

	const canWrite = $derived(['admin', 'operator'].includes(getSession()?.role ?? ''));
	const repositoryNames = $derived(new Map(repositories.map((item) => [item.id, item.name])));
	const agentOptions = $derived<SelectOption[]>([
		{ value: '', label: t('profiles.previewHost') },
		...agents
			.filter((agent) => agent.status === 'active')
			.map((agent) => ({ value: agent.id, label: agent.hostname }))
	]);

	const columns = $derived<TableColumn<Profile>[]>([
		{ key: 'name', header: t('profiles.profile'), value: (row) => row.name, render: nameCell },
		{
			key: 'repository',
			header: t('profiles.repository'),
			value: (row) => repositoryNames.get(row.repository_id) ?? row.repository_id,
			render: repositoryCell
		},
		{ key: 'tags', header: t('profiles.tags'), value: (row) => row.tags.join(', ') },
		{
			key: 'retention',
			header: t('profiles.retention'),
			value: (row) => retentionSummary(row),
			render: retentionCell
		}
	]);

	const empty = $derived(!loading && !error && profiles.length === 0);

	function openCreate(): void {
		void goto(resolve('/(app)/profiles/new'));
	}

	function openEdit(profile: Profile): void {
		void goto(resolve('/(app)/profiles/[id]', { id: profile.id }));
	}

	function actionsFor(profile: Profile): MenuItem[] {
		const items: MenuItem[] = [
			{ label: t('profiles.preview'), icon: 'eye', onSelect: () => openPreview(profile) }
		];
		if (canWrite) {
			items.push({
				label: t('profiles.edit'),
				icon: 'settings',
				onSelect: () => openEdit(profile)
			});
			items.push({
				label: t('profiles.delete'),
				icon: 'octagon',
				tone: 'danger',
				onSelect: () => {
					deleteTarget = profile;
					deleteOpen = true;
				}
			});
		}
		return items;
	}

	function retentionSummary(profile: Profile): string {
		if (!profile.retention) return t('common.none');
		const retention = profile.retention;
		const parts: string[] = [];
		if ((retention.keep_last ?? 0) > 0)
			parts.push(t('profiles.retention.summary.last', { count: retention.keep_last ?? 0 }));
		if ((retention.keep_daily ?? 0) > 0)
			parts.push(t('profiles.retention.summary.daily', { count: retention.keep_daily ?? 0 }));
		if ((retention.keep_weekly ?? 0) > 0)
			parts.push(t('profiles.retention.summary.weekly', { count: retention.keep_weekly ?? 0 }));
		if ((retention.keep_monthly ?? 0) > 0)
			parts.push(t('profiles.retention.summary.monthly', { count: retention.keep_monthly ?? 0 }));
		if ((retention.keep_yearly ?? 0) > 0)
			parts.push(t('profiles.retention.summary.yearly', { count: retention.keep_yearly ?? 0 }));
		if (retention.keep_within)
			parts.push(t('profiles.retention.summary.within', { value: retention.keep_within }));
		return parts.length > 0 ? parts.join(', ') : t('common.none');
	}

	function messageOf(problem: unknown): string {
		if (problem instanceof ApiError) return tErrorCode(problem.code) ?? problem.title;
		return t('table.error');
	}

	function problemOf(problem: unknown): ApiProblem | null {
		if (problem instanceof ApiError) return problem.problem;
		if (problem && typeof problem === 'object' && 'problem' in problem) {
			return (problem as { problem: ApiProblem }).problem;
		}
		return null;
	}

	async function load(): Promise<void> {
		loading = true;
		error = null;
		try {
			const [profileList, repositoryList, agentList] = await Promise.all([
				service.listProfiles(),
				service.listRepositories(),
				service.listAgents()
			]);
			profiles = profileList;
			repositories = repositoryList;
			agents = agentList;
		} catch (problem) {
			error = messageOf(problem);
		} finally {
			loading = false;
		}
	}

	onMount(load);

	async function confirmDelete(): Promise<void> {
		if (!deleteTarget) return;
		deleting = true;
		try {
			await service.deleteProfile(deleteTarget.id);
			pushToast('success', t('profiles.deleted'));
			deleteOpen = false;
			deleteTarget = null;
			await load();
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		} finally {
			deleting = false;
		}
	}

	function openPreview(profile: Profile): void {
		previewTarget = profile;
		previewAgentId = '';
		previewResult = null;
		previewProblem = null;
		previewOpen = true;
	}

	async function runPreview(): Promise<void> {
		if (!previewTarget || !previewAgentId) return;
		previewLoading = true;
		previewProblem = null;
		previewResult = null;
		try {
			previewResult = await service.previewProfile(previewTarget.id, previewAgentId);
		} catch (problem) {
			previewProblem = problemOf(problem);
		} finally {
			previewLoading = false;
		}
	}
</script>

{#snippet nameCell(profile: Profile)}
	<span class="profile">
		<a class="profile__name" href={resolve('/(app)/profiles/[id]', { id: profile.id })}>
			{profile.name}
		</a>
		<span class="profile__meta">{profile.tags.join(' · ')}</span>
	</span>
{/snippet}

{#snippet repositoryCell(profile: Profile)}
	<span>{repositoryNames.get(profile.repository_id) ?? profile.repository_id}</span>
{/snippet}

{#snippet retentionCell(profile: Profile)}
	<span class="profile__meta">{retentionSummary(profile)}</span>
{/snippet}

<svelte:head>
	<title>{t('profiles.title')} · {t('app.name')}</title>
</svelte:head>

<PageHeader
	title={t('profiles.title')}
	description={t('profiles.description')}
	count={loading ? undefined : profiles.length}
>
	{#if canWrite}
		<Button variant="primary" icon="plus" onclick={openCreate}>{t('profiles.add')}</Button>
	{/if}
</PageHeader>

{#if empty}
	<EmptyState
		icon="checklist"
		title={t('profiles.emptyTitle')}
		description={t('profiles.emptyDescription')}
		bordered
	>
		{#if canWrite}
			<Button variant="primary" onclick={openCreate}>{t('profiles.addFirst')}</Button>
		{/if}
	</EmptyState>
{:else}
	<Table
		{columns}
		rows={profiles}
		{loading}
		{error}
		rowKey={(profile) => profile.id}
		filterPlaceholder={t('profiles.searchPlaceholder')}
	>
		{#snippet rowActions(profile)}
			<RowActions
				moreLabel={t('profiles.moreActions', { name: profile.name })}
				items={actionsFor(profile)}
			/>
		{/snippet}
	</Table>
{/if}

<SidePanel
	bind:open={previewOpen}
	title={t('profiles.previewTitle')}
	subtitle={t('profiles.previewDescription')}
>
	<div class="preview">
		<Select
			id="preview-host"
			label={t('profiles.previewHost')}
			options={agentOptions}
			bind:value={previewAgentId}
			onchange={runPreview}
		/>
		{#if previewProblem}
			<Alert tone="danger" title={tErrorCode(previewProblem.code) ?? previewProblem.title}>
				{previewProblem.field}{previewProblem.reason ? `: ${previewProblem.reason}` : ''}
			</Alert>
		{/if}
		{#if previewLoading}
			<p class="preview__hint">{t('common.loading')}</p>
		{:else if previewResult}
			{#each previewResult.steps as step (step.kind + step.command)}
				<div class="step">
					<div class="step__head">
						<span class="step__kind">{step.kind}</span>
						{#if step.maintenance}
							<Badge tone="info">{t('profiles.previewMaintenance')}</Badge>
						{/if}
					</div>
					<div class="step__label">{t('profiles.previewCommand')}</div>
					<pre class="step__command">{step.command}</pre>
					{#if step.environment.length > 0}
						<div class="step__label">{t('profiles.previewEnvironment')}</div>
						<ul class="step__env">
							{#each step.environment as entry (entry.name)}
								<li><span class="step__env-name">{entry.name}</span> = {entry.value}</li>
							{/each}
						</ul>
					{/if}
				</div>
			{/each}
		{:else}
			<p class="preview__hint">{t('profiles.previewEmpty')}</p>
		{/if}
	</div>
</SidePanel>

<ConfirmDialog
	bind:open={deleteOpen}
	title={t('profiles.deleteTitle')}
	message={t('profiles.deleteBody', { name: deleteTarget?.name ?? '' })}
	confirmLabel={t('profiles.delete')}
	tone="danger"
	loading={deleting}
	onConfirm={confirmDelete}
/>

<style>
	.profile {
		display: flex;
		flex-direction: column;
	}

	.profile__name {
		color: var(--rh-color-text);
		font-weight: var(--rh-font-weight-semibold);
		white-space: nowrap;
	}

	.profile__name:hover {
		color: var(--rh-color-link);
	}

	.profile__meta {
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.preview {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-4);
	}

	.preview__hint {
		color: var(--rh-color-text-muted);
	}

	.step {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-2);
		padding: var(--rh-space-3);
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
	}

	.step__head {
		display: flex;
		align-items: center;
		gap: var(--rh-space-2);
	}

	.step__kind {
		font-weight: var(--rh-font-weight-semibold);
	}

	.step__label {
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.step__command {
		margin: 0;
		padding: var(--rh-space-2);
		overflow-x: auto;
		border-radius: var(--rh-radius-sm);
		background-color: var(--rh-color-surface-sunken, var(--rh-color-control-bg));
		font-family: var(--rh-font-mono);
		font-size: var(--rh-font-size-sm);
		white-space: pre-wrap;
		word-break: break-all;
	}

	.step__env {
		margin: 0;
		padding-left: var(--rh-space-4);
		font-family: var(--rh-font-mono);
		font-size: var(--rh-font-size-sm);
	}

	.step__env-name {
		color: var(--rh-color-text-muted);
	}
</style>
