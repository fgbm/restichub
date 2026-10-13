<script lang="ts">
	import { onMount } from 'svelte';
	import { ApiError, fieldError, namesField, service } from '#lib/api/index.ts';
	import type { ApiProblem, Tenant } from '#lib/api/index.ts';
	import { Alert, Button, Dialog, PageHeader, Table, TextField } from '#lib/components/ui/index.ts';
	import type { TableColumn } from '#lib/components/ui/index.ts';
	import { getLocale, t, tErrorCode } from '#lib/i18n/index.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';

	let tenants = $state<Tenant[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);

	let createOpen = $state(false);
	let newName = $state('');
	let creating = $state(false);
	let createError = $state<string | null>(null);
	let createProblem = $state<ApiProblem | null>(null);

	// The fields of the create form. A problem of another field (for example `body`) shows the
	// general alert instead of the message next to an input.
	const createFields = ['name'];

	let deleteTarget = $state<Tenant | null>(null);
	let deleteOpen = $state(false);
	let deleteTyped = $state('');
	let deleting = $state(false);
	let busyId = $state<string | null>(null);

	function formatDate(value: string): string {
		return new Date(value).toLocaleString(getLocale());
	}

	function statusLabel(status: Tenant['status']): string {
		if (status === 'active') return t('common.active');
		if (status === 'suspended') return t('common.suspended');
		return t('common.deleted');
	}

	function statusBadge(status: Tenant['status']): 'success' | 'warning' | 'neutral' {
		if (status === 'active') return 'success';
		if (status === 'suspended') return 'warning';
		return 'neutral';
	}

	const columns = $derived<TableColumn<Tenant>[]>([
		{ key: 'name', header: t('tenants.name'), mono: true },
		{
			key: 'status',
			header: t('common.status'),
			cell: (row) => statusLabel(row.status),
			badge: (row) => statusBadge(row.status)
		},
		{
			key: 'user_count',
			header: t('tenants.users'),
			cell: (row) => String(row.user_count)
		},
		{
			key: 'admin_state',
			header: t('users.role.admin'),
			cell: (row) => (row.admin_state === 'none' ? t('tenants.noAdministrator') : ''),
			badge: (row) => (row.admin_state === 'none' ? 'warning' : null)
		},
		{ key: 'created_at', header: t('common.createdAt'), cell: (row) => formatDate(row.created_at) }
	]);

	async function load(): Promise<void> {
		loading = true;
		error = null;
		try {
			tenants = await service.listTenants();
		} catch (problem) {
			error = messageOf(problem);
		} finally {
			loading = false;
		}
	}

	function messageOf(problem: unknown): string {
		if (problem instanceof ApiError) return tErrorCode(problem.code) ?? problem.title;
		return t('table.error');
	}

	onMount(load);

	function openCreate(): void {
		newName = '';
		createError = null;
		createProblem = null;
		createOpen = true;
	}

	async function createTenant(): Promise<void> {
		if (!newName) return;
		creating = true;
		createError = null;
		createProblem = null;
		try {
			await service.createTenant({ name: newName });
			pushToast('success', t('tenants.created'));
			createOpen = false;
			newName = '';
			await load();
		} catch (problem) {
			createProblem = problem instanceof ApiError ? problem.problem : null;
			createError = namesField(createProblem, createFields) ? null : messageOf(problem);
		} finally {
			creating = false;
		}
	}

	async function toggleStatus(tenant: Tenant): Promise<void> {
		busyId = tenant.id;
		try {
			if (tenant.status === 'active') {
				await service.suspendTenant(tenant.id);
				pushToast('success', t('tenants.suspended'));
			} else {
				await service.resumeTenant(tenant.id);
				pushToast('success', t('tenants.resumed'));
			}
			await load();
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		} finally {
			busyId = null;
		}
	}

	async function confirmDelete(): Promise<void> {
		if (!deleteTarget || deleteTyped !== deleteTarget.name) return;
		deleting = true;
		try {
			await service.deleteTenant(deleteTarget.id);
			pushToast('success', t('tenants.deleted'));
			deleteTarget = null;
			deleteOpen = false;
			await load();
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		} finally {
			deleting = false;
		}
	}
</script>

<svelte:head>
	<title>{t('tenants.title')} · {t('app.name')}</title>
</svelte:head>

<PageHeader
	title={t('tenants.title')}
	description={t('tenants.description')}
	count={loading ? undefined : tenants.length}
>
	<Button variant="primary" icon="plus" onclick={openCreate}>{t('tenants.create')}</Button>
</PageHeader>

<Table
	{columns}
	rows={tenants}
	{loading}
	{error}
	rowKey={(tenant) => tenant.id}
	filterPlaceholder={t('tenants.searchPlaceholder')}
>
	{#snippet rowActions(tenant)}
		{#if tenant.status !== 'deleted'}
			<Button
				size="sm"
				variant="ghost"
				loading={busyId === tenant.id}
				onclick={() => toggleStatus(tenant)}
			>
				{tenant.status === 'active' ? t('tenants.suspend') : t('tenants.resume')}
			</Button>
			<Button
				size="sm"
				variant="ghost"
				disabled={tenant.status !== 'suspended'}
				onclick={() => {
					deleteTarget = tenant;
					deleteTyped = '';
					deleteOpen = true;
				}}
			>
				{t('common.delete')}
			</Button>
		{/if}
	{/snippet}
</Table>

<Dialog bind:open={createOpen} title={t('tenants.createTitle')} onsubmit={createTenant}>
	{#if createError}
		<Alert variant="inline" tone="danger" title={createError} />
	{/if}
	<TextField
		id="tenant-name"
		label={t('tenants.name')}
		error={fieldError(createProblem, 'name')}
		bind:value={newName}
		placeholder={t('tenants.namePlaceholder')}
		mono
		required
	/>
	{#snippet footer()}
		<Button onclick={() => (createOpen = false)}>{t('common.cancel')}</Button>
		<Button type="submit" variant="primary" loading={creating} disabled={!newName}>
			{t('tenants.create')}
		</Button>
	{/snippet}
</Dialog>

<Dialog
	bind:open={deleteOpen}
	size="lg"
	icon="octagon"
	title={t('tenants.deleteTitle')}
	onsubmit={confirmDelete}
>
	<p>
		{t('tenants.deleteBody', { name: deleteTarget?.name ?? '' })}
	</p>
	<TextField
		id="tenant-delete-confirm"
		label={t('tenants.deleteConfirm', { name: deleteTarget?.name ?? '' })}
		bind:value={deleteTyped}
		placeholder={t('tenants.namePlaceholder')}
		mono
		autocomplete="off"
	/>
	{#snippet footer()}
		<Button onclick={() => (deleteOpen = false)}>{t('common.cancel')}</Button>
		<Button
			type="submit"
			variant="danger"
			loading={deleting}
			disabled={!deleteTarget || deleteTyped !== deleteTarget.name}
		>
			{t('tenants.delete')}
		</Button>
	{/snippet}
</Dialog>
