<script lang="ts">
	import { onMount } from 'svelte';
	import { ApiError, assignableRoles, fieldError, namesField, service } from '#lib/api/index.ts';
	import type { ApiProblem, ApiToken, ApiTokenCreated, Role } from '#lib/api/index.ts';
	import { SettingsLayout } from '#lib/components/index.ts';
	import {
		Alert,
		Button,
		CopyField,
		Dialog,
		KeyValue,
		DataTable,
		FilterBar,
		Select,
		StatusPill,
		TextField
	} from '#lib/components/ui/index.ts';
	import type {
		FilterFacet,
		KeyValueItem,
		RowAction,
		SelectOption,
		TableColumn
	} from '#lib/components/ui/index.ts';
	import { matchesSearch } from '#lib/filters.ts';
	import { queryFilters } from '#lib/stores/query-filters.svelte.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import { formatTime } from '#lib/format.ts';
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
	let newExpiry = $state('never');
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

	// The least privilege comes first and is the default.
	const roles = $derived([...assignableRoles()].reverse());

	const roleOptions = $derived<SelectOption[]>(
		roles.map((role) => ({
			value: role,
			label: roleKeys[role]
		}))
	);

	const expiryOptions = $derived<SelectOption[]>([
		{ value: 'never', label: t('tokens.expiry.never') },
		{ value: '30', label: t('tokens.expiry.days', { days: 30 }) },
		{ value: '90', label: t('tokens.expiry.days', { days: 90 }) },
		{ value: '365', label: t('tokens.expiry.year') }
	]);

	function formatDate(value: string | null | undefined): string {
		return value ? formatTime(value) : t('common.never');
	}

	const columns = $derived<TableColumn<ApiToken>[]>([
		{ key: 'name', header: t('tokens.name'), truncate: 'sm' },
		{ key: 'display_prefix', header: t('tokens.prefix'), mono: true, sortable: false },
		{
			key: 'role',
			header: t('users.role'),
			cell: (row) => roleKeys[row.role],
			badge: () => 'neutral'
		},
		{ key: 'created_at', header: t('common.createdAt'), date: 'relative' },
		{
			key: 'expires_at',
			header: t('tokens.expiresAt'),
			date: 'relative',
			value: (row) => row.expires_at ?? '',
			cell: () => t('common.never')
		},
		{
			key: 'last_used_at',
			header: t('tokens.lastUsed'),
			date: 'relative',
			value: (row) => row.last_used_at ?? '',
			cell: () => t('common.never')
		},
		{
			key: 'revoked_at',
			header: t('tokens.state'),
			value: (row) => (row.revoked_at ? t('tokens.state.revoked') : t('tokens.state.active')),
			render: stateCell
		}
	]);

	const filters = queryFilters(['q', 'state'] as const);

	const facets = $derived<FilterFacet[]>([
		{
			key: 'state',
			label: t('tokens.state'),
			multiple: false,
			options: (['active', 'revoked'] as const).map((value) => ({
				value,
				label: t(value === 'active' ? 'tokens.state.active' : 'tokens.state.revoked'),
				count: tokens.filter((token) => Boolean(token.revoked_at) === (value === 'revoked')).length
			}))
		}
	]);

	const shown = $derived(
		tokens.filter((token) => {
			const { q, state } = filters.value;
			if (state && Boolean(token.revoked_at) !== (state === 'revoked')) return false;
			return matchesSearch(q, [token.name, token.display_prefix]);
		})
	);

	function rowActions(token: ApiToken): RowAction[] {
		return [
			{
				id: 'revoke',
				label: t('tokens.revoke'),
				icon: 'circle-slash',
				hidden: Boolean(token.revoked_at),
				onSelect: () => {
					revokeTarget = token;
					revokeOpen = true;
				}
			}
		];
	}

	const createdItems = $derived<KeyValueItem[]>(
		createdToken
			? [
					{ key: 'name', label: t('tokens.name'), value: createdToken.name },
					{ key: 'role', label: t('users.role'), value: roleKeys[createdToken.role] },
					{
						key: 'expires',
						label: t('tokens.expiresAt'),
						value: formatDate(createdToken.expires_at)
					}
				]
			: []
	);

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
		newRole = roles[0];
		newExpiry = 'never';
		createError = null;
		createProblem = null;
		createdToken = null;
		createOpen = true;
	}

	function expiresAt(choice: string): string | null {
		if (choice === 'never') return null;
		return new Date(Date.now() + Number(choice) * 24 * 60 * 60 * 1000).toISOString();
	}

	async function createToken(): Promise<void> {
		if (!newName) return;
		creating = true;
		createError = null;
		createProblem = null;
		try {
			createdToken = await service.createToken({
				name: newName,
				role: newRole,
				expires_at: expiresAt(newExpiry)
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

{#snippet stateCell(token: ApiToken)}
	<StatusPill tone={token.revoked_at ? 'neutral' : 'success'}>
		{token.revoked_at ? t('tokens.state.revoked') : t('tokens.state.active')}
	</StatusPill>
{/snippet}

<SettingsLayout
	title={t('tokens.title')}
	description={t('tokens.description')}
	count={loading ? undefined : tokens.length}
>
	{#snippet actions()}
		<Button variant="primary" icon="plus" onclick={openCreate}>{t('tokens.create')}</Button>
	{/snippet}

	<DataTable
		{columns}
		rows={shown}
		{loading}
		{error}
		rowKey={(token) => token.id}
		detailTitle={(token) => token.name}
		actions={rowActions}
		emptyTitle={filters.value.q || filters.value.state ? t('filter.noMatches') : undefined}
	>
		{#snippet toolbar()}
			<FilterBar
				label={t('tokens.filters')}
				value={filters.value}
				search={{ key: 'q', label: t('hosts.search'), placeholder: t('tokens.searchPlaceholder') }}
				{facets}
				onchange={filters.update}
				onreset={filters.reset}
			/>
		{/snippet}
	</DataTable>

	<Alert variant="inline" icon="lock" title={t('tokens.valueNote')} />
</SettingsLayout>

<Dialog
	bind:open={createOpen}
	size={createdToken ? 'lg' : 'md'}
	title={createdToken ? t('tokens.valueOnceTitle') : t('tokens.createTitle')}
	onsubmit={createdToken ? closeCreate : createToken}
>
	{#if createdToken}
		<Alert tone="attention" icon="key" title={t('tokens.valueOnceBody')}>
			{t('tokens.valueOnceText')}
		</Alert>
		<CopyField value={createdToken.token} label={t('tokens.value')} secret />
		<KeyValue items={createdItems} />
	{:else}
		{#if createError}
			<Alert variant="inline" tone="danger" title={createError} />
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
		<Select
			id="token-expiry"
			label={t('tokens.expiresAt')}
			error={fieldError(createProblem, 'expires_at')}
			bind:value={newExpiry}
			options={expiryOptions}
		/>
	{/if}
	{#snippet footer()}
		{#if createdToken}
			<Button type="submit" variant="primary">{t('common.done')}</Button>
		{:else}
			<Button onclick={closeCreate}>{t('common.cancel')}</Button>
			<Button type="submit" variant="primary" loading={creating} disabled={!newName}>
				{t('tokens.create')}
			</Button>
		{/if}
	{/snippet}
</Dialog>

<Dialog bind:open={revokeOpen} size="sm" icon="octagon" title={t('tokens.revokeTitle')}>
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
