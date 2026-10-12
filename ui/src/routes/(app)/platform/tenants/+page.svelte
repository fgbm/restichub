<script lang="ts">
	import { onMount } from 'svelte';
	import { ApiError, service } from '#lib/api/index.ts';
	import type { Tenant } from '#lib/api/index.ts';
	import {
		Button,
		DataTable,
		Dialog,
		FormField,
		InlineAlert,
		PageHeader,
		TextField
	} from '#lib/components/index.ts';
	import type { Column } from '#lib/components/index.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';

	let tenants = $state<Tenant[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);

	let createOpen = $state(false);
	let newName = $state('');
	let newAdminUsername = $state('');
	let newAdminDisplayName = $state('');
	let newAdminPassword = $state('');
	let creating = $state(false);
	let createError = $state<string | null>(null);

	let deleteTarget = $state<Tenant | null>(null);
	let deleteOpen = $state(false);
	let deleting = $state(false);
	let busyId = $state<string | null>(null);

	const columns = $derived<Column<Tenant>[]>([
		{ key: 'name', header: t('tenants.name'), mono: true },
		{
			key: 'status',
			header: t('common.status'),
			cell: (row) => (row.status === 'active' ? t('common.active') : t('common.suspended')),
			badge: (row) => (row.status === 'active' ? 'success' : 'warning')
		},
		{ key: 'created_at', header: t('common.createdAt'), mono: true }
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

	async function createTenant(): Promise<void> {
		creating = true;
		createError = null;
		try {
			await service.createTenant({
				name: newName,
				first_admin: {
					username: newAdminUsername,
					display_name: newAdminDisplayName,
					password: newAdminPassword
				}
			});
			pushToast('success', t('tenants.created'));
			createOpen = false;
			newName = '';
			newAdminUsername = '';
			newAdminDisplayName = '';
			newAdminPassword = '';
			await load();
		} catch (problem) {
			createError = messageOf(problem);
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
		if (!deleteTarget) return;
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

<PageHeader title={t('tenants.title')} description={t('tenants.description')}>
	<Button variant="primary" onclick={() => (createOpen = true)}>{t('tenants.create')}</Button>
</PageHeader>

<DataTable {columns} rows={tenants} {loading} {error} rowKey={(tenant) => tenant.id}>
	{#snippet rowActions(tenant)}
		<div class="row-actions">
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
				onclick={() => {
					deleteTarget = tenant;
					deleteOpen = true;
				}}
			>
				{t('common.delete')}
			</Button>
		</div>
	{/snippet}
</DataTable>

<Dialog bind:open={createOpen} title={t('tenants.createTitle')}>
	{#if createError}
		<InlineAlert tone="danger" title={createError} />
	{/if}
	<FormField label={t('tenants.name')} for="tenant-name" required>
		<TextField
			id="tenant-name"
			bind:value={newName}
			placeholder={t('tenants.namePlaceholder')}
			mono
		/>
	</FormField>
	<FormField label={t('tenants.adminUsername')} for="tenant-admin-username" required>
		<TextField id="tenant-admin-username" bind:value={newAdminUsername} mono autocomplete="off" />
	</FormField>
	<FormField label={t('tenants.adminDisplayName')} for="tenant-admin-display-name" required>
		<TextField id="tenant-admin-display-name" bind:value={newAdminDisplayName} autocomplete="off" />
	</FormField>
	<FormField
		label={t('tenants.adminPassword')}
		for="tenant-admin-password"
		hint={t('setup.passwordHint')}
		required
	>
		<TextField
			id="tenant-admin-password"
			type="password"
			bind:value={newAdminPassword}
			autocomplete="new-password"
		/>
	</FormField>
	<div class="dialog-actions">
		<Button onclick={() => (createOpen = false)}>{t('common.cancel')}</Button>
		<Button
			variant="primary"
			loading={creating}
			disabled={!newName ||
				!newAdminUsername ||
				!newAdminDisplayName ||
				newAdminPassword.length < 12}
			onclick={createTenant}
		>
			{t('common.create')}
		</Button>
	</div>
</Dialog>

<Dialog bind:open={deleteOpen} title={t('tenants.deleteTitle')}>
	<p>
		{t('tenants.deleteBody', { name: deleteTarget?.name ?? '' })}
	</p>
	<div class="dialog-actions">
		<Button onclick={() => (deleteOpen = false)}>{t('common.cancel')}</Button>
		<Button variant="danger" loading={deleting} onclick={confirmDelete}>
			{t('common.delete')}
		</Button>
	</div>
</Dialog>

<style>
	.row-actions {
		display: inline-flex;
		gap: var(--rh-space-1);
	}

	.dialog-actions {
		display: flex;
		justify-content: flex-end;
		gap: var(--rh-space-2);
	}
</style>
