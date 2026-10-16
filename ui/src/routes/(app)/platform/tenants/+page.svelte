<script lang="ts">
	import { onMount } from 'svelte';
	import { ApiError, fieldError, namesField, service } from '#lib/api/index.ts';
	import type { ApiProblem, Tenant } from '#lib/api/index.ts';
	import { Page } from '#lib/components/layout/index.ts';
	import {
		Alert,
		Button,
		Dialog,
		FilterBar,
		KeyValue,
		ListView,
		SidePanel,
		TextField
	} from '#lib/components/ui/index.ts';
	import type { KeyValueItem, ListMeta, RowAction } from '#lib/components/ui/index.ts';
	import { matchesSearch } from '#lib/filters.ts';
	import { plural } from '#lib/list-format.ts';
	import { iconOf, pillOf, tenantState } from '#lib/vocabulary.ts';
	import { queryFilters } from '#lib/stores/query-filters.svelte.ts';
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

	const filters = queryFilters(['q', 'status', 'sort', 'order'] as const);

	const matching = $derived(
		tenants.filter((tenant) => matchesSearch(filters.value.q, [tenant.name]))
	);
	const deletedCount = $derived(matching.filter((tenant) => tenant.status === 'deleted').length);

	const states = $derived({
		key: 'status',
		default: 'active',
		options: [
			...(['active', 'suspended'] as const).map((value) => ({
				value,
				label: t(value === 'active' ? 'tenants.tab.active' : 'tenants.tab.suspended'),
				count: matching.filter((tenant) => tenant.status === value).length
			})),
			...(deletedCount > 0 || filters.value.status === 'deleted'
				? [{ value: 'deleted', label: t('tenants.tab.deleted'), count: deletedCount }]
				: [])
		]
	});

	const sort = $derived({
		default: 'name',
		options: [{ value: 'name', label: t('tenants.sort.name') }]
	});

	const shown = $derived.by(() => {
		const status = filters.value.status || 'active';
		const factor = filters.value.order === 'desc' ? -1 : 1;
		return matching
			.filter((tenant) => tenant.status === status)
			.sort((a, b) => factor * a.name.localeCompare(b.name));
	});

	function resetFilters(): void {
		filters.update({ q: '', status: '' });
	}

	function meta(tenant: Tenant): ListMeta[] {
		return [
			plural('tenants.userCount', tenant.user_count),
			plural('tenants.hostCount', tenant.agent_count),
			{
				text: t('tenants.meta.created', {
					date: new Date(tenant.created_at).toLocaleDateString(getLocale())
				}),
				time: tenant.created_at
			}
		];
	}

	let detail = $state<Tenant | null>(null);
	let detailOpen = $state(false);

	function openDetail(tenant: Tenant): void {
		detail = tenant;
		detailOpen = true;
	}

	const detailItems = $derived<KeyValueItem[]>(
		detail
			? [
					{ key: 'status', label: t('common.status'), value: statusLabel(detail.status) },
					{ key: 'users', label: t('tenants.users'), value: String(detail.user_count) },
					{
						key: 'admin',
						label: t('users.role.admin'),
						value: detail.admin_state === 'none' ? t('tenants.noAdministrator') : t('common.yes')
					},
					{ key: 'created', label: t('common.createdAt'), value: formatDate(detail.created_at) }
				]
			: []
	);

	function rowActions(tenant: Tenant): RowAction[] {
		if (tenant.status === 'deleted') return [];
		return [
			{
				id: 'status',
				label: tenant.status === 'active' ? t('tenants.suspend') : t('tenants.resume'),
				icon: tenant.status === 'active' ? 'pause' : 'play',
				disabled: busyId === tenant.id,
				onSelect: () => toggleStatus(tenant)
			},
			{
				id: 'delete',
				label: t('common.delete'),
				icon: 'trash',
				danger: true,
				disabled: tenant.status !== 'suspended',
				onSelect: () => {
					deleteTarget = tenant;
					deleteTyped = '';
					deleteOpen = true;
				}
			}
		];
	}

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

<Page
	title={t('tenants.title')}
	documentTitle={`${t('tenants.title')} · ${t('nav.platformMode')}`}
	description={t('tenants.description')}
	count={loading ? undefined : tenants.length}
>
	{#snippet actions()}
		<Button variant="primary" icon="plus" onclick={openCreate}>{t('tenants.create')}</Button>
	{/snippet}

	<ListView
		label={t('tenants.title')}
		items={shown}
		itemKey={(tenant) => tenant.id}
		icon={(tenant) => iconOf(tenantState(tenant))}
		pill={(tenant) => pillOf(tenantState(tenant))}
		title={(tenant) => tenant.name}
		onOpen={openDetail}
		{meta}
		actions={rowActions}
		moreLabel={(tenant) => t('tenants.moreActions', { name: tenant.name })}
		{loading}
		{error}
		filtered={tenants.length > 0}
		onReset={resetFilters}
		emptyIcon="building-2"
		emptyTitle={t('tenants.emptyTitle')}
	>
		{#snippet toolbar()}
			<FilterBar
				label={t('tenants.filters')}
				value={filters.value}
				search={{ key: 'q', label: t('hosts.search'), placeholder: t('tenants.searchPlaceholder') }}
				{states}
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

<style>
</style>
