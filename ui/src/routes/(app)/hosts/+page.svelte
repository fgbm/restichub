<script lang="ts">
	import { onMount } from 'svelte';
	import { ApiError, fieldError, service } from '#lib/api/index.ts';
	import type { Agent, EnrollmentTokenCreated, ApiProblem } from '#lib/api/index.ts';
	import {
		Alert,
		Button,
		CopyField,
		Dialog,
		PageHeader,
		Table,
		TextField
	} from '#lib/components/ui/index.ts';
	import type { TableColumn } from '#lib/components/ui/index.ts';
	import { getLocale, t, tErrorCode } from '#lib/i18n/index.ts';
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
	const installCommand = $derived(
		t('hosts.installPlaceholder', { token: createdToken?.token ?? '<token>' })
	);

	function formatDate(value: string | null | undefined): string {
		if (!value) return t('common.never');
		return new Date(value).toLocaleString(getLocale());
	}

	const columns = $derived<TableColumn<Agent>[]>([
		{ key: 'hostname', header: t('hosts.hostname') },
		{ key: 'os', header: t('hosts.os') },
		{ key: 'arch', header: t('hosts.arch') },
		{ key: 'agent_version', header: t('hosts.version'), mono: true },
		{
			key: 'status',
			header: t('hosts.status'),
			cell: (row) => t(row.status === 'revoked' ? 'hosts.status.revoked' : 'hosts.status.active'),
			badge: (row) => (row.status === 'revoked' ? 'danger' : 'success')
		},
		{
			key: 'last_seen_at',
			header: t('hosts.lastSeen'),
			cell: (row) => formatDate(row.last_seen_at)
		},
		{ key: 'created_at', header: t('hosts.created'), cell: (row) => formatDate(row.created_at) }
	]);

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

<svelte:head>
	<title>{t('hosts.title')} · {t('app.name')}</title>
</svelte:head>

<PageHeader title={t('hosts.title')} description={t('hosts.description')}>
	{#if canWrite}
		<Button variant="primary" onclick={openAdd}>{t('hosts.add')}</Button>
	{/if}
</PageHeader>

<Table
	{columns}
	rows={agents}
	{loading}
	{error}
	rowKey={(agent) => agent.id}
	emptyTitle={t('hosts.emptyTitle')}
	emptyDescription={t('hosts.emptyDescription')}
>
	{#snippet rowActions(agent)}
		{#if canWrite && agent.status === 'active'}
			<Button
				size="sm"
				variant="ghost"
				onclick={() => {
					revokeTarget = agent;
					revokeOpen = true;
				}}
			>
				{t('hosts.revoke')}
			</Button>
		{/if}
	{/snippet}
</Table>

<Dialog bind:open={addOpen} title={createdToken ? t('hosts.tokenTitle') : t('hosts.addTitle')}>
	{#if createdToken}
		<Alert tone="attention" title={t('hosts.tokenBody')} />
		<CopyField value={createdToken.token} label={t('hosts.tokenValue')} />
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
			<Button variant="primary" onclick={closeAdd}>{t('common.close')}</Button>
		{:else}
			<Button onclick={closeAdd}>{t('common.cancel')}</Button>
			<Button variant="primary" loading={creating} onclick={createToken}>
				{t('hosts.createToken')}
			</Button>
		{/if}
	{/snippet}
</Dialog>

<Dialog bind:open={revokeOpen} title={t('hosts.revokeTitle')}>
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
