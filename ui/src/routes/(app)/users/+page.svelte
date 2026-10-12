<script lang="ts">
	import { onMount } from 'svelte';
	import { ApiError, assignableRoles, service } from '#lib/api/index.ts';
	import type { Role, User } from '#lib/api/index.ts';
	import {
		Button,
		DataTable,
		FormField,
		InlineAlert,
		PageHeader,
		Select,
		SidePanel,
		TextField
	} from '#lib/components/index.ts';
	import type { Column, SelectOption } from '#lib/components/index.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';

	let users = $state<User[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);

	let createOpen = $state(false);
	let creating = $state(false);
	let createError = $state<string | null>(null);
	let busyId = $state<string | null>(null);

	let newUsername = $state('');
	let newDisplayName = $state('');
	let newRole = $state<Role>('viewer');
	let newPassword = $state('');

	const roleKeys = $derived<Record<Role, string>>({
		platform_admin: t('users.role.platformAdmin'),
		admin: t('users.role.admin'),
		operator: t('users.role.operator'),
		viewer: t('users.role.viewer')
	});

	const roleOptions = $derived<SelectOption[]>(
		assignableRoles(getSession()?.tenant?.id).map((role) => ({
			value: role,
			label: roleKeys[role]
		}))
	);

	const columns = $derived<Column<User>[]>([
		{ key: 'username', header: t('users.username'), mono: true },
		{ key: 'display_name', header: t('users.displayName') },
		{
			key: 'role',
			header: t('users.role'),
			cell: (row) => roleKeys[row.role],
			badge: (row) => (row.role === 'admin' ? 'info' : 'neutral')
		},
		{
			key: 'disabled',
			header: t('common.status'),
			cell: (row) => (row.disabled ? t('common.disabled') : t('common.enabled')),
			badge: (row) => (row.disabled ? 'warning' : 'success')
		}
	]);

	function messageOf(problem: unknown): string {
		if (problem instanceof ApiError) return tErrorCode(problem.code) ?? problem.title;
		return t('table.error');
	}

	async function load(): Promise<void> {
		loading = true;
		error = null;
		try {
			users = await service.listUsers();
		} catch (problem) {
			error = messageOf(problem);
		} finally {
			loading = false;
		}
	}

	onMount(load);

	function openCreate(): void {
		newUsername = '';
		newDisplayName = '';
		newRole = assignableRoles(getSession()?.tenant?.id)[0];
		newPassword = '';
		createError = null;
		createOpen = true;
	}

	async function createUser(): Promise<void> {
		creating = true;
		createError = null;
		try {
			await service.createUser({
				username: newUsername,
				display_name: newDisplayName,
				role: newRole,
				password: newPassword
			});
			pushToast('success', t('users.created'));
			createOpen = false;
			await load();
		} catch (problem) {
			createError = messageOf(problem);
		} finally {
			creating = false;
		}
	}

	async function toggleDisabled(user: User): Promise<void> {
		busyId = user.id;
		try {
			await service.updateUser(user.id, { disabled: !user.disabled });
			pushToast('success', user.disabled ? t('users.enabledToast') : t('users.disabledToast'));
			await load();
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		} finally {
			busyId = null;
		}
	}
</script>

<PageHeader title={t('users.title')} description={t('users.description')}>
	<Button variant="primary" onclick={openCreate}>{t('users.create')}</Button>
</PageHeader>

<DataTable {columns} rows={users} {loading} {error} rowKey={(user) => user.id}>
	{#snippet rowActions(user)}
		<Button
			size="sm"
			variant="ghost"
			loading={busyId === user.id}
			onclick={() => toggleDisabled(user)}
		>
			{user.disabled ? t('users.enable') : t('users.disable')}
		</Button>
	{/snippet}
</DataTable>

<SidePanel bind:open={createOpen} title={t('users.createTitle')}>
	{#if createError}
		<InlineAlert tone="danger" title={createError} />
	{/if}
	<FormField label={t('users.username')} for="user-username" required>
		<TextField id="user-username" bind:value={newUsername} mono autocomplete="off" />
	</FormField>
	<FormField label={t('users.displayName')} for="user-display-name" required>
		<TextField id="user-display-name" bind:value={newDisplayName} autocomplete="off" />
	</FormField>
	<FormField label={t('users.role')} for="user-role" required>
		<Select id="user-role" bind:value={newRole} options={roleOptions} />
	</FormField>
	<FormField
		label={t('setup.password')}
		for="user-password"
		hint={t('setup.passwordHint')}
		required
	>
		<TextField
			id="user-password"
			type="password"
			bind:value={newPassword}
			autocomplete="new-password"
		/>
	</FormField>
	<div class="panel-actions">
		<Button onclick={() => (createOpen = false)}>{t('common.cancel')}</Button>
		<Button
			variant="primary"
			loading={creating}
			disabled={!newUsername || !newDisplayName || newPassword.length < 12}
			onclick={createUser}
		>
			{t('common.create')}
		</Button>
	</div>
</SidePanel>

<style>
	.panel-actions {
		display: flex;
		justify-content: flex-end;
		gap: var(--rh-space-2);
		margin-top: var(--rh-space-2);
	}
</style>
