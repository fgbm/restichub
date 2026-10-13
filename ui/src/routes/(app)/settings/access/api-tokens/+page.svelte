<script lang="ts">
	import { onMount } from 'svelte';
	import { ApiError, assignableRoles, fieldError, namesField, service } from '#lib/api/index.ts';
	import type { ApiProblem, ApiToken, ApiTokenCreated, Role } from '#lib/api/index.ts';
	import {
		Alert,
		Button,
		CopyField,
		Dialog,
		PageHeader,
		Select,
		Table,
		TextField
	} from '#lib/components/ui/index.ts';
	import type { SelectOption, TableColumn } from '#lib/components/ui/index.ts';
	import { getLocale, t, tErrorCode } from '#lib/i18n/index.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';

	let tokens = $state<ApiToken[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);

	let createOpen = $state(false);
	let creating = $state(false);
	let createError = $state<string | null>(null);
	let createProblem = $state<ApiProblem | null>(null);
	let newName = $state('');

	// The fields of the create form. A problem of another field (for example `body`) shows the
	// general alert instead of the message next to an input.
	const createFields = ['name', 'role', 'expires_at'];
	let newRole = $state<Role>('viewer');
	let newExpiry = $state('');
	let createdToken = $state<ApiTokenCreated | null>(null);

	let revokeTarget = $state<ApiToken | null>(null);
	let revokeOpen = $state(false);
	let revoking = $state(false);

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

	function formatDate(value: string | null | undefined): string {
		if (!value) return t('common.never');
		return new Date(value).toLocaleString(getLocale());
	}

	const columns = $derived<TableColumn<ApiToken>[]>([
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
		newRole = assignableRoles(getSession()?.role)[0];
		newExpiry = '';
		createError = null;
		createProblem = null;
		createdToken = null;
		createOpen = true;
	}

	async function createToken(): Promise<void> {
		creating = true;
		createError = null;
		createProblem = null;
		try {
			createdToken = await service.createToken({
				name: newName,
				role: newRole,
				expires_at: newExpiry ? new Date(newExpiry).toISOString() : null
			});
			pushToast('success', t('tokens.created'));
			await load();
		} catch (problem) {
			createProblem = problem instanceof ApiError ? problem.problem : null;
			createError = namesField(createProblem, createFields) ? null : messageOf(problem);
		} finally {
			creating = false;
		}
	}

	function closeCreate(): void {
		createOpen = false;
		createdToken = null;
	}

	// The token value is shown once. Clear it whenever the dialog closes, also
	// with the Escape key or a click on the backdrop.
	$effect(() => {
		if (!createOpen) createdToken = null;
	});

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

<svelte:head>
	<title>{t('tokens.title')} · {t('app.name')}</title>
</svelte:head>

<PageHeader title={t('tokens.title')} description={t('tokens.description')}>
	<Button variant="primary" onclick={openCreate}>{t('tokens.create')}</Button>
</PageHeader>

<Table {columns} rows={tokens} {loading} {error} rowKey={(token) => token.id}>
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
</Table>

<Dialog
	bind:open={createOpen}
	title={createdToken ? t('tokens.valueOnceTitle') : t('tokens.createTitle')}
>
	{#if createdToken}
		<Alert tone="attention" title={t('tokens.valueOnceBody')} />
		<CopyField value={createdToken.token} label={t('tokens.value')} />
	{:else}
		{#if createError}
			<Alert tone="danger" title={createError} />
		{/if}
		<TextField
			id="token-name"
			label={t('tokens.name')}
			error={fieldError(createProblem, 'name')}
			bind:value={newName}
			placeholder={t('tokens.namePlaceholder')}
			required
		/>
		<Select
			id="token-role"
			label={t('users.role')}
			error={fieldError(createProblem, 'role')}
			bind:value={newRole}
			options={roleOptions}
			required
		/>
		<TextField
			id="token-expiry"
			label={t('tokens.expiresAt')}
			error={fieldError(createProblem, 'expires_at')}
			type="date"
			bind:value={newExpiry}
		/>
	{/if}
	{#snippet footer()}
		{#if createdToken}
			<Button variant="primary" onclick={closeCreate}>{t('common.close')}</Button>
		{:else}
			<Button onclick={closeCreate}>{t('common.cancel')}</Button>
			<Button variant="primary" loading={creating} disabled={!newName} onclick={createToken}>
				{t('common.create')}
			</Button>
		{/if}
	{/snippet}
</Dialog>

<Dialog bind:open={revokeOpen} title={t('tokens.revokeTitle')}>
	<p>
		{t('tokens.revokeBody', { name: revokeTarget?.name ?? '' })}
	</p>
	{#snippet footer()}
		<Button onclick={() => (revokeOpen = false)}>{t('common.cancel')}</Button>
		<Button variant="danger" loading={revoking} onclick={confirmRevoke}>
			{t('tokens.revoke')}
		</Button>
	{/snippet}
</Dialog>
