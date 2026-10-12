<script lang="ts">
	import { onMount } from 'svelte';
	import { ApiError, assignableRoles, service } from '#lib/api/index.ts';
	import type { ApiToken, ApiTokenCreated, Role } from '#lib/api/index.ts';
	import {
		Button,
		CopyField,
		DataTable,
		Dialog,
		FormField,
		InlineAlert,
		PageHeader,
		Select,
		TextField
	} from '#lib/components/index.ts';
	import type { Column, SelectOption } from '#lib/components/index.ts';
	import { getLocale, t, tErrorCode } from '#lib/i18n/index.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';

	let tokens = $state<ApiToken[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);

	let createOpen = $state(false);
	let creating = $state(false);
	let createError = $state<string | null>(null);
	let newName = $state('');
	let newRole = $state<Role>('viewer');
	let newExpiry = $state('');
	let createdToken = $state<ApiTokenCreated | null>(null);

	let revokeTarget = $state<ApiToken | null>(null);
	let revokeOpen = $state(false);
	let revoking = $state(false);

	const roleKeys: Record<Role, string> = {
		platform_admin: t('users.role.platformAdmin'),
		admin: t('users.role.admin'),
		operator: t('users.role.operator'),
		viewer: t('users.role.viewer')
	};

	const roleOptions = $derived<SelectOption[]>(
		assignableRoles(getSession()?.tenant?.id).map((role) => ({
			value: role,
			label: roleKeys[role]
		}))
	);

	function formatDate(value: string | null | undefined): string {
		if (!value) return t('common.never');
		return new Date(value).toLocaleString(getLocale());
	}

	const columns = $derived<Column<ApiToken>[]>([
		{ key: 'name', header: t('tokens.name') },
		{
			key: 'role',
			header: t('users.role'),
			cell: (row) => roleKeys[row.role],
			badge: (row) => (row.role === 'admin' ? 'info' : 'neutral')
		},
		{ key: 'display_prefix', header: t('tokens.prefix'), mono: true },
		{ key: 'created_at', header: t('common.createdAt'), cell: (row) => formatDate(row.created_at) },
		{ key: 'expires_at', header: t('tokens.expiresAt'), cell: (row) => formatDate(row.expires_at) },
		{
			key: 'last_used_at',
			header: t('tokens.lastUsed'),
			cell: (row) => formatDate(row.last_used_at)
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
			tokens = await service.listTokens();
		} catch (problem) {
			error = messageOf(problem);
		} finally {
			loading = false;
		}
	}

	onMount(load);

	function openCreate(): void {
		newName = '';
		newRole = assignableRoles(getSession()?.tenant?.id)[0];
		newExpiry = '';
		createError = null;
		createdToken = null;
		createOpen = true;
	}

	async function createToken(): Promise<void> {
		creating = true;
		createError = null;
		try {
			createdToken = await service.createToken({
				name: newName,
				role: newRole,
				expires_at: newExpiry ? new Date(newExpiry).toISOString() : null
			});
			pushToast('success', t('tokens.created'));
			await load();
		} catch (problem) {
			createError = messageOf(problem);
		} finally {
			creating = false;
		}
	}

	function closeCreate(): void {
		createOpen = false;
		createdToken = null;
	}

	async function confirmRevoke(): Promise<void> {
		if (!revokeTarget) return;
		revoking = true;
		try {
			await service.revokeToken(revokeTarget.id);
			pushToast('success', t('tokens.revoked'));
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

<PageHeader title={t('tokens.title')} description={t('tokens.description')}>
	<Button variant="primary" onclick={openCreate}>{t('tokens.create')}</Button>
</PageHeader>

<DataTable {columns} rows={tokens} {loading} {error} rowKey={(token) => token.id}>
	{#snippet rowActions(token)}
		<Button
			size="sm"
			variant="ghost"
			onclick={() => {
				revokeTarget = token;
				revokeOpen = true;
			}}
		>
			{t('tokens.revoke')}
		</Button>
	{/snippet}
</DataTable>

<Dialog
	bind:open={createOpen}
	title={createdToken ? t('tokens.valueOnceTitle') : t('tokens.createTitle')}
>
	{#if createdToken}
		<InlineAlert tone="warning" title={t('tokens.valueOnceBody')} />
		<CopyField value={createdToken.token} label={t('tokens.value')} />
		<div class="dialog-actions">
			<Button variant="primary" onclick={closeCreate}>{t('common.close')}</Button>
		</div>
	{:else}
		{#if createError}
			<InlineAlert tone="danger" title={createError} />
		{/if}
		<FormField label={t('tokens.name')} for="token-name" required>
			<TextField id="token-name" bind:value={newName} placeholder={t('tokens.namePlaceholder')} />
		</FormField>
		<FormField label={t('users.role')} for="token-role" required>
			<Select id="token-role" bind:value={newRole} options={roleOptions} />
		</FormField>
		<FormField label={t('tokens.expiresAt')} for="token-expiry">
			<TextField id="token-expiry" type="date" bind:value={newExpiry} />
		</FormField>
		<div class="dialog-actions">
			<Button onclick={closeCreate}>{t('common.cancel')}</Button>
			<Button variant="primary" loading={creating} disabled={!newName} onclick={createToken}>
				{t('common.create')}
			</Button>
		</div>
	{/if}
</Dialog>

<Dialog bind:open={revokeOpen} title={t('tokens.revokeTitle')}>
	<p>
		{t('tokens.revokeBody', { name: revokeTarget?.name ?? '' })}
	</p>
	<div class="dialog-actions">
		<Button onclick={() => (revokeOpen = false)}>{t('common.cancel')}</Button>
		<Button variant="danger" loading={revoking} onclick={confirmRevoke}>
			{t('tokens.revoke')}
		</Button>
	</div>
</Dialog>

<style>
	.dialog-actions {
		display: flex;
		justify-content: flex-end;
		gap: var(--rh-space-2);
	}
</style>
