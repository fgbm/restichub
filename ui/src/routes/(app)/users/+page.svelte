<script lang="ts">
	import { onMount } from 'svelte';
	import { ApiError, assignableRoles, fieldError, namesField, service } from '#lib/api/index.ts';
	import type { ApiProblem, Role, User } from '#lib/api/index.ts';
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
	let createProblem = $state<ApiProblem | null>(null);
	let busyId = $state<string | null>(null);

	let resetOpen = $state(false);
	let resetUser = $state<User | null>(null);
	let resetting = $state(false);
	let resetError = $state<string | null>(null);
	let newResetPassword = $state('');

	// The fields of the create form. A problem of another field (for example `body`) shows the
	// general alert instead of the message next to an input.
	const createFields = ['username', 'display_name', 'role', 'password'];

	// The create endpoint answers HTTP 409 `username_taken` without a field name.
	const usernameTaken = $derived(createProblem?.code === 'username_taken');

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
		assignableRoles(getSession()?.role).map((role) => ({
			value: role,
			label: roleKeys[role]
		}))
	);

	// A shared account belongs to another tenant; the label tells a tenant Admin that it cannot
	// reset the password or change the display name (ADR-0025).
	const columns = $derived<Column<User>[]>([
		{
			key: 'username',
			header: t('users.username'),
			mono: true,
			cell: (row) => (row.shared ? `${row.username} \u00b7 ${t('users.shared')}` : row.username)
		},
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
		newRole = assignableRoles(getSession()?.role)[0];
		newPassword = '';
		createError = null;
		createProblem = null;
		createOpen = true;
	}

	async function createUser(): Promise<void> {
		creating = true;
		createError = null;
		createProblem = null;
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
			createProblem = problem instanceof ApiError ? problem.problem : null;
			// A taken username is a problem of the username field (ADR-0025).
			createError =
				usernameTaken || namesField(createProblem, createFields) ? null : messageOf(problem);
		} finally {
			creating = false;
		}
	}

	function openReset(user: User): void {
		resetUser = user;
		newResetPassword = '';
		resetError = null;
		resetOpen = true;
	}

	async function resetPassword(): Promise<void> {
		if (!resetUser) return;
		resetting = true;
		resetError = null;
		try {
			await service.resetUserPassword(resetUser.id, { new_password: newResetPassword });
			pushToast('success', t('users.resetDone'));
			resetOpen = false;
			await load();
		} catch (problem) {
			resetError = messageOf(problem);
		} finally {
			resetting = false;
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
			disabled={user.shared}
			title={user.shared ? t('users.sharedResetHint') : t('users.resetPassword')}
			aria-label={t('users.resetPassword')}
			onclick={() => openReset(user)}
		>
			{t('users.resetPassword')}
		</Button>
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
	<FormField
		label={t('users.username')}
		for="user-username"
		error={usernameTaken ? tErrorCode('username_taken') : fieldError(createProblem, 'username')}
		required
	>
		<TextField
			id="user-username"
			bind:value={newUsername}
			invalid={usernameTaken || Boolean(fieldError(createProblem, 'username'))}
			mono
			autocomplete="off"
		/>
	</FormField>
	<FormField
		label={t('users.displayName')}
		for="user-display-name"
		error={fieldError(createProblem, 'display_name')}
		required
	>
		<TextField
			id="user-display-name"
			bind:value={newDisplayName}
			invalid={Boolean(fieldError(createProblem, 'display_name'))}
			autocomplete="off"
		/>
	</FormField>
	<FormField
		label={t('users.role')}
		for="user-role"
		error={fieldError(createProblem, 'role')}
		required
	>
		<Select
			id="user-role"
			bind:value={newRole}
			options={roleOptions}
			invalid={Boolean(fieldError(createProblem, 'role'))}
		/>
	</FormField>
	<FormField
		label={t('setup.password')}
		for="user-password"
		hint={t('setup.passwordHint')}
		error={fieldError(createProblem, 'password')}
		required
	>
		<TextField
			id="user-password"
			type="password"
			bind:value={newPassword}
			invalid={Boolean(fieldError(createProblem, 'password'))}
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

<SidePanel bind:open={resetOpen} title={t('users.resetTitle')}>
	{#if resetError}
		<InlineAlert tone="danger" title={resetError} />
	{/if}
	<FormField
		label={t('setup.password')}
		for="user-reset-password"
		hint={t('setup.passwordHint')}
		required
	>
		<TextField
			id="user-reset-password"
			type="password"
			bind:value={newResetPassword}
			autocomplete="new-password"
		/>
	</FormField>
	<div class="panel-actions">
		<Button onclick={() => (resetOpen = false)}>{t('common.cancel')}</Button>
		<Button
			variant="primary"
			loading={resetting}
			disabled={newResetPassword.length < 12}
			onclick={resetPassword}
		>
			{t('users.resetPassword')}
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
