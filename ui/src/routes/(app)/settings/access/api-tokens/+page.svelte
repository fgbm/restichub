<script lang="ts">
	import { onMount } from 'svelte';
	import { ApiError, assignableRoles, fieldError, namesField, service } from '#lib/api/index.ts';
	import type { ApiProblem, ApiToken, ApiTokenCreated, Role } from '#lib/api/index.ts';
	import { Page } from '#lib/components/layout/index.ts';
	import {
		Alert,
		Button,
		CopyField,
		Dialog,
		KeyValue,
		FilterBar,
		ListView,
		Select,
		SidePanel,
		TextField
	} from '#lib/components/ui/index.ts';
	import type {
		FilterFacet,
		KeyValueItem,
		ListMeta,
		RowAction,
		SelectOption
	} from '#lib/components/ui/index.ts';
	import { facetCounts, matchesSearch, splitList } from '#lib/filters.ts';
	import { emptyError, focusFirstInvalid } from '#lib/form.ts';
	import { ago, expiry } from '#lib/list-format.ts';
	import { apiTokenState, iconOf, pillOf } from '#lib/vocabulary.ts';
	import { queryFilters } from '#lib/stores/query-filters.svelte.ts';
	import { hasRole } from '#lib/stores/session.svelte.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import { formatTime } from '#lib/format.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';

	// An Admin creates and revokes the API tokens (ADR-0023, operation `api_tokens_write`).
	const canWrite = $derived(hasRole('admin', 'platform_admin'));

	let tokens = $state<ApiToken[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);

	let createOpen = $state(false);
	let creating = $state(false);
	let createError = $state<string | null>(null);
	let createProblem = $state<ApiProblem | null>(null);
	let newName = $state('');
	let createSubmitted = $state(false);

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

	const filters = queryFilters(['q', 'role', 'state', 'sort', 'order'] as const);

	const facets = $derived<FilterFacet[]>([
		{
			key: 'role',
			label: t('users.role'),
			options: facetCounts(tokens, (token) => [token.role]).map((option) => ({
				...option,
				label: roleKeys[option.value as Role] ?? option.value
			}))
		}
	]);

	function valid(token: ApiToken): boolean {
		const state = apiTokenState(token).key;
		return state === 'active' || state === 'expiring';
	}

	const matching = $derived(
		tokens.filter((token) => {
			const { q, role } = filters.value;
			if (role && !splitList(role).includes(token.role)) return false;
			return matchesSearch(q, [token.name, token.display_prefix]);
		})
	);

	const states = $derived({
		default: 'valid',
		options: [
			{ value: 'valid', label: t('tokens.tab.valid'), count: matching.filter(valid).length },
			{
				value: 'ended',
				label: t('tokens.tab.ended'),
				count: matching.filter((token) => !valid(token)).length
			}
		]
	});

	const sort = $derived({
		default: 'name',
		options: [
			{ value: 'name', label: t('tokens.sort.name') },
			{ value: 'used', label: t('tokens.sort.used'), order: 'desc' as const },
			{ value: 'expires', label: t('tokens.sort.expires') }
		]
	});

	const shown = $derived.by(() => {
		const ended = filters.value.state === 'ended';
		const list = matching.filter((token) => valid(token) !== ended);
		const field = filters.value.sort || 'name';
		const order = filters.value.order || (field === 'used' ? 'desc' : 'asc');
		const factor = order === 'desc' ? -1 : 1;
		const key = (token: ApiToken) =>
			field === 'used'
				? (token.last_used_at ?? '')
				: field === 'expires'
					? (token.expires_at ?? '9999')
					: token.name;
		return [...list].sort((a, b) => factor * key(a).localeCompare(key(b)));
	});

	const filtered = $derived(tokens.length > 0 && Boolean(filters.value.q || filters.value.role));

	function resetFilters(): void {
		filters.update({ q: '', role: '', state: '' });
	}

	function meta(token: ApiToken): ListMeta[] {
		const end = token.expires_at ? expiry(token.expires_at) : null;
		return [
			{ text: `${token.display_prefix}…`, mono: true },
			roleKeys[token.role],
			token.last_used_at
				? {
						text: t('tokens.meta.used', { time: ago(token.last_used_at) }),
						time: token.last_used_at
					}
				: t('tokens.meta.neverUsed'),
			token.revoked_at
				? null
				: end
					? { text: end.text, time: token.expires_at ?? undefined }
					: t('tokens.meta.noExpiry')
		];
	}

	let detail = $state<ApiToken | null>(null);
	let detailOpen = $state(false);

	function openDetail(token: ApiToken): void {
		detail = token;
		detailOpen = true;
	}

	const detailItems = $derived<KeyValueItem[]>(
		detail
			? [
					{ key: 'prefix', label: t('tokens.prefix'), value: detail.display_prefix, mono: true },
					{ key: 'role', label: t('users.role'), value: roleKeys[detail.role] },
					{ key: 'state', label: t('tokens.state'), value: apiTokenState(detail).label },
					{ key: 'created', label: t('common.createdAt'), value: formatTime(detail.created_at) },
					{ key: 'expires', label: t('tokens.expiresAt'), value: formatDate(detail.expires_at) },
					{ key: 'used', label: t('tokens.lastUsed'), value: formatDate(detail.last_used_at) },
					...(detail.revoked_at
						? [
								{
									key: 'revoked',
									label: t('tokens.state.revoked'),
									value: formatTime(detail.revoked_at)
								}
							]
						: [])
				]
			: []
	);

	function rowActions(token: ApiToken): RowAction[] {
		if (!canWrite) return [];
		return [
			{
				id: 'revoke',
				label: t('tokens.revoke'),
				icon: 'ban',
				danger: true,
				hidden: !valid(token),
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
		createSubmitted = false;
		createOpen = true;
	}

	function expiresAt(choice: string): string | null {
		if (choice === 'never') return null;
		return new Date(Date.now() + Number(choice) * 24 * 60 * 60 * 1000).toISOString();
	}

	async function createToken(): Promise<void> {
		createSubmitted = true;
		if (focusFirstInvalid([['token-name', newName === '']])) return;
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

<Page
	title={t('tokens.title')}
	description={t('tokens.description')}
	count={loading ? undefined : tokens.length}
>
	{#snippet actions()}
		{#if canWrite}
			<Button variant="primary" icon="plus" onclick={openCreate}>{t('tokens.create')}</Button>
		{/if}
	{/snippet}

	<ListView
		label={t('tokens.title')}
		items={shown}
		itemKey={(token) => token.id}
		icon={(token) => iconOf(apiTokenState(token))}
		pill={(token) => pillOf(apiTokenState(token))}
		title={(token) => token.name}
		onOpen={openDetail}
		{meta}
		actions={rowActions}
		{loading}
		{error}
		{filtered}
		onReset={resetFilters}
		emptyIcon="key"
		emptyTitle={t('tokens.emptyTitle')}
	>
		{#snippet toolbar()}
			<FilterBar
				label={t('tokens.filters')}
				value={filters.value}
				search={{ key: 'q', label: t('hosts.search'), placeholder: t('tokens.searchPlaceholder') }}
				{states}
				{facets}
				{sort}
				onchange={filters.update}
				onreset={resetFilters}
			/>
		{/snippet}
	</ListView>

	<SidePanel bind:open={detailOpen} title={detail?.name ?? ''}>
		<KeyValue items={detailItems} stacked />
	</SidePanel>
</Page>

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
			error={emptyError(createSubmitted, newName) ?? fieldError(createProblem, 'name')}
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
			<Button type="submit" variant="primary" loading={creating}>
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
