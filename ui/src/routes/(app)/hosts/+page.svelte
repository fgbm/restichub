<script lang="ts">
	import { onMount } from 'svelte';
	import { ApiError, fieldError, service } from '#lib/api/index.ts';
	import type { Agent, EnrollmentTokenCreated, ApiProblem } from '#lib/api/index.ts';
	import {
		Alert,
		Button,
		CopyField,
		Dialog,
		DropdownMenu,
		EmptyState,
		PageHeader,
		StatusPill,
		Table,
		TextField
	} from '#lib/components/ui/index.ts';
	import type { TableColumn } from '#lib/components/ui/index.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import { formatTime, relativeTime } from '#lib/format.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';

	let agents = $state<Agent[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);

	let addOpen = $state(false);
	let creating = $state(false);
	let createError = $state<string | null>(null);
	let createProblem = $state<ApiProblem | null>(null);
	let labels = $state('');
	let createdToken = $state<EnrollmentTokenCreated | null>(null);

	let revokeTarget = $state<Agent | null>(null);
	let revokeOpen = $state(false);
	let revoking = $state(false);

	const canWrite = $derived(getSession()?.role === 'admin');
	// The registration command carries this installation URL (ADR-0032).
	let origin = $state('');
	onMount(() => {
		origin = window.location.origin;
	});
	const installCommand = $derived(
		t('hosts.installPlaceholder', { url: origin, token: createdToken?.token ?? '<token>' })
	);

	const columns = $derived<TableColumn<Agent>[]>([
		{
			key: 'hostname',
			header: t('hosts.host'),
			value: (row) => `${row.hostname} ${row.os} ${row.arch}`,
			render: hostCell
		},
		{
			key: 'status',
			header: t('hosts.status'),
			value: (row) => statusLabel(row),
			render: statusCell
		},
		{ key: 'agent_version', header: t('hosts.agent'), render: versionCell },
		{
			key: 'last_seen_at',
			header: t('hosts.lastSeen'),
			value: (row) => row.last_seen_at ?? '',
			render: lastSeenCell
		},
		{ key: 'created_at', header: t('hosts.created'), render: createdCell }
	]);

	function statusLabel(agent: Agent): string {
		return t(agent.status === 'revoked' ? 'hosts.status.revoked' : 'hosts.status.active');
	}

	const empty = $derived(!loading && !error && agents.length === 0);

	function messageOf(problem: unknown): string {
		if (problem instanceof ApiError) return tErrorCode(problem.code) ?? problem.title;
		return t('table.error');
	}

	async function load(): Promise<void> {
		loading = true;
		error = null;
		try {
			agents = await service.listAgents();
		} catch (problem) {
			error = messageOf(problem);
		} finally {
			loading = false;
		}
	}

	onMount(load);

	function openAdd(): void {
		labels = '';
		createError = null;
		createProblem = null;
		createdToken = null;
		addOpen = true;
	}

	async function createToken(): Promise<void> {
		creating = true;
		createError = null;
		createProblem = null;
		try {
			const parsed = labels
				.split(',')
				.map((label) => label.trim())
				.filter((label) => label.length > 0);
			createdToken = await service.createEnrollmentToken({
				labels: parsed.length > 0 ? parsed : undefined
			});
			pushToast('success', t('hosts.tokenCreated'));
		} catch (problem) {
			createProblem = problem instanceof ApiError ? problem.problem : null;
			createError = fieldError(createProblem, 'labels') ? null : messageOf(problem);
		} finally {
			creating = false;
		}
	}

	function closeAdd(): void {
		addOpen = false;
		createdToken = null;
	}

	// The token value is shown once. Clear it whenever the dialog closes, also
	// with the Escape key or a click on the backdrop.
	$effect(() => {
		if (!addOpen) createdToken = null;
	});

	async function confirmRevoke(): Promise<void> {
		if (!revokeTarget) return;
		revoking = true;
		try {
			await service.revokeAgent(revokeTarget.id);
			pushToast('success', t('hosts.revoked'));
			revokeOpen = false;
			revokeTarget = null;
			await load();
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		} finally {
			revoking = false;
		}
	}
</script>

{#snippet timeCell(value: string | null | undefined)}
	{#if value}
		<time datetime={value} title={formatTime(value)}>{relativeTime(value)}</time>
	{:else}
		{t('common.never')}
	{/if}
{/snippet}

{#snippet lastSeenCell(agent: Agent)}
	{@render timeCell(agent.last_seen_at)}
{/snippet}

{#snippet createdCell(agent: Agent)}
	{@render timeCell(agent.created_at)}
{/snippet}

{#snippet hostCell(agent: Agent)}
	<span class="host">
		<span class="host__name">{agent.hostname}</span>
		<span class="host__meta">{agent.os} · {agent.arch}</span>
	</span>
{/snippet}

{#snippet statusCell(agent: Agent)}
	<StatusPill tone={agent.status === 'revoked' ? 'neutral' : 'success'}>
		{statusLabel(agent)}
	</StatusPill>
{/snippet}

{#snippet versionCell(agent: Agent)}
	<span class="host__version">{agent.agent_version}</span>
{/snippet}

<svelte:head>
	<title>{t('hosts.title')} · {t('app.name')}</title>
</svelte:head>

<PageHeader
	title={t('hosts.title')}
	description={t('hosts.description')}
	count={loading ? undefined : agents.length}
	bordered
>
	{#if canWrite}
		<Button variant="primary" icon="plus" onclick={openAdd}>{t('hosts.add')}</Button>
	{/if}
</PageHeader>

<div class="hosts">
	{#if empty}
		<EmptyState
			icon="host"
			title={t('hosts.emptyTitle')}
			description={t('hosts.emptyDescription')}
			bordered
		>
			{#if canWrite}
				<Button variant="primary" onclick={openAdd}>{t('hosts.addFirst')}</Button>
			{/if}
		</EmptyState>
	{:else}
		<Table
			{columns}
			rows={agents}
			{loading}
			{error}
			rowKey={(agent) => agent.id}
			filterPlaceholder={t('hosts.searchPlaceholder')}
		>
			{#snippet rowActions(agent)}
				{#if canWrite && agent.status === 'active'}
					<DropdownMenu
						icon="more"
						align="end"
						label={t('hosts.moreActions', { name: agent.hostname })}
						items={[
							{
								label: t('hosts.revokeAgent'),
								icon: 'octagon',
								tone: 'danger',
								onSelect: () => {
									revokeTarget = agent;
									revokeOpen = true;
								}
							}
						]}
					/>
				{/if}
			{/snippet}
		</Table>
	{/if}
</div>

<Dialog
	bind:open={addOpen}
	size={createdToken ? 'lg' : 'md'}
	title={createdToken ? t('hosts.tokenTitle') : t('hosts.addTitle')}
	description={createdToken ? undefined : t('hosts.addDescription')}
	onsubmit={createdToken ? closeAdd : createToken}
>
	{#if createdToken}
		<Alert tone="attention" icon="key" title={t('tokens.valueOnceBody')}>
			{t('hosts.tokenNote')}
		</Alert>
		<CopyField value={createdToken.token} label={t('hosts.tokenValue')} secret />
		<CopyField value={installCommand} label={t('hosts.installCommand')} />
	{:else}
		{#if createError}
			<Alert tone="danger" title={createError} />
		{/if}
		<TextField
			id="host-labels"
			label={t('hosts.labels')}
			error={fieldError(createProblem, 'labels')}
			bind:value={labels}
			placeholder={t('hosts.labelsPlaceholder')}
		/>
	{/if}
	{#snippet footer()}
		{#if createdToken}
			<Button type="submit" variant="primary">{t('common.done')}</Button>
		{:else}
			<Button onclick={closeAdd}>{t('common.cancel')}</Button>
			<Button type="submit" variant="primary" loading={creating}>
				{t('hosts.createToken')}
			</Button>
		{/if}
	{/snippet}
</Dialog>

<Dialog bind:open={revokeOpen} size="sm" icon="octagon" title={t('hosts.revokeTitle')}>
	<p>
		{t('hosts.revokeBody', { name: revokeTarget?.hostname ?? '' })}
	</p>
	{#snippet footer()}
		<Button onclick={() => (revokeOpen = false)}>{t('common.cancel')}</Button>
		<Button variant="danger" loading={revoking} onclick={confirmRevoke}>
			{t('hosts.revoke')}
		</Button>
	{/snippet}
</Dialog>

<style>
	.hosts {
		margin-top: var(--rh-space-4);
	}

	.host {
		display: flex;
		flex-direction: column;
	}

	.host__name {
		font-weight: var(--rh-font-weight-semibold);
		white-space: nowrap;
	}

	.host__meta {
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.host__version {
		font-family: var(--rh-font-mono);
		font-size: var(--rh-font-size-sm);
	}
</style>
