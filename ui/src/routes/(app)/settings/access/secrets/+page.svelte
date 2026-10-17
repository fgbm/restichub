<script lang="ts">
	import { onMount } from 'svelte';
	import { resolve } from '$app/paths';
	import { ApiError, fieldError, namesField, service } from '#lib/api/index.ts';
	import type { ApiProblem, Secret } from '#lib/api/index.ts';
	import { SettingsLayout } from '#lib/components/index.ts';
	import {
		Alert,
		Button,
		Dialog,
		FilterBar,
		KeyValue,
		ListView,
		PasswordField,
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
	import type { SecretUsageKind } from '#lib/api/index.ts';
	import { formatTime } from '#lib/format.ts';
	import { changedAt, plural } from '#lib/list-format.ts';
	import { iconOf, pillOf, secretState } from '#lib/vocabulary.ts';
	import { facetCounts, matchesSearch, splitList } from '#lib/filters.ts';
	import { emptyError, focusFirstInvalid } from '#lib/form.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import { queryFilters } from '#lib/stores/query-filters.svelte.ts';
	import {
		CREATABLE_PURPOSES,
		USAGE_KEYS,
		canReplaceValue,
		purposeKey,
		usageHref,
		wasReplaced
	} from '#lib/secrets.ts';
	import type { CreatablePurpose } from '#lib/secrets.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';

	const canWrite = $derived(['admin', 'platform_admin'].includes(getSession()?.role ?? ''));

	let secrets = $state<Secret[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);

	let createOpen = $state(false);
	let creating = $state(false);
	let createError = $state<string | null>(null);
	let createProblem = $state<ApiProblem | null>(null);
	let newName = $state('');
	let newPurpose = $state<CreatablePurpose>('hook_secret');
	let newValue = $state('');
	let createSubmitted = $state(false);
	const createFields = ['name', 'purpose', 'value'];

	let replaceTarget = $state<Secret | null>(null);
	let replaceOpen = $state(false);
	let replacing = $state(false);
	let replaceError = $state<string | null>(null);
	let replaceProblem = $state<ApiProblem | null>(null);
	let replaceValue = $state('');
	let replaceSubmitted = $state(false);

	let deleteTarget = $state<Secret | null>(null);
	let deleteOpen = $state(false);
	let deleting = $state(false);

	const purposeOptions = $derived<SelectOption[]>(
		CREATABLE_PURPOSES.map((value) => ({ value, label: t(`secrets.purpose.${value}`) }))
	);

	function purposeLabel(purpose: string): string {
		const key = purposeKey(purpose);
		return key ? t(key) : purpose;
	}

	const filters = queryFilters(['q', 'purpose', 'state', 'sort', 'order'] as const);

	const facets = $derived<FilterFacet[]>([
		{
			key: 'purpose',
			label: t('secrets.purpose'),
			options: facetCounts(secrets, (secret) => [secret.purpose]).map((option) => ({
				...option,
				label: purposeLabel(option.value)
			}))
		}
	]);

	const matching = $derived(
		secrets.filter((secret) => {
			const { q, purpose } = filters.value;
			if (purpose && !splitList(purpose).includes(secret.purpose)) return false;
			return matchesSearch(q, [secret.name, ...secret.used_by.map((usage) => usage.name)]);
		})
	);

	const states = $derived({
		default: 'all',
		options: [
			{ value: 'all', label: t('secrets.tab.all'), count: matching.length },
			{
				value: 'unused',
				label: t('secrets.tab.unused'),
				count: matching.filter((secret) => secret.used_by.length === 0).length
			}
		]
	});

	const sort = $derived({
		default: 'name',
		options: [{ value: 'name', label: t('secrets.sort.name') }]
	});

	const shown = $derived.by(() => {
		const list = matching.filter(
			(secret) => filters.value.state !== 'unused' || secret.used_by.length === 0
		);
		const factor = filters.value.order === 'desc' ? -1 : 1;
		return [...list].sort((a, b) => factor * a.name.localeCompare(b.name));
	});

	const filtered = $derived(
		secrets.length > 0 && Boolean(filters.value.q || filters.value.purpose || filters.value.state)
	);

	function resetFilters(): void {
		filters.update({ q: '', purpose: '', state: '' });
	}

	const USAGE_COUNTS: Record<SecretUsageKind, string> = {
		repository: 'secrets.count.repository',
		profile: 'secrets.count.profile',
		channel: 'secrets.count.channel',
		job_template: 'secrets.count.jobTemplate'
	};

	// The users by kind, without links: the details panel links each one.
	function usageText(secret: Secret): string {
		if (secret.used_by.length === 0) return t('secrets.meta.unused');
		const counts: Partial<Record<SecretUsageKind, number>> = {};
		for (const usage of secret.used_by) counts[usage.kind] = (counts[usage.kind] ?? 0) + 1;
		const parts = (Object.keys(USAGE_COUNTS) as SecretUsageKind[])
			.filter((kind) => counts[kind])
			.map((kind) => plural(USAGE_COUNTS[kind], counts[kind] ?? 0));
		return t('secrets.meta.used', { list: parts.join(', ') });
	}

	function meta(secret: Secret): ListMeta[] {
		const at = wasReplaced(secret) ? secret.updated_at : secret.created_at;
		return [
			purposeLabel(secret.purpose),
			usageText(secret),
			{
				text: t(wasReplaced(secret) ? 'secrets.meta.replaced' : 'secrets.meta.created', {
					time: changedAt(at)
				}),
				time: at
			}
		];
	}

	let detail = $state<Secret | null>(null);
	let detailOpen = $state(false);

	function openDetail(secret: Secret): void {
		detail = secret;
		detailOpen = true;
	}

	const detailItems = $derived<KeyValueItem[]>(
		detail
			? [
					{ key: 'purpose', label: t('secrets.purpose'), value: purposeLabel(detail.purpose) },
					{
						key: 'used',
						label: t('secrets.usedBy'),
						value: detail.used_by.length === 0 ? t('secrets.unused') : '',
						render: detail.used_by.length > 0 ? detailUsers : undefined
					},
					{ key: 'created', label: t('common.createdAt'), value: formatTime(detail.created_at) },
					{
						key: 'replaced',
						label: t('secrets.replacedAt'),
						value: wasReplaced(detail) ? formatTime(detail.updated_at) : t('common.never')
					}
				]
			: []
	);

	function rowActions(secret: Secret): RowAction[] {
		if (!canWrite) return [];
		return [
			{
				id: 'replace',
				label: t('secrets.replace'),
				icon: 'replace',
				hidden: !canReplaceValue(secret),
				onSelect: () => openReplace(secret)
			},
			{
				id: 'delete',
				label: t('secrets.delete'),
				hint: secret.used_by.length > 0 ? t('secrets.deleteInUse') : undefined,
				icon: 'trash',
				danger: true,
				disabled: secret.used_by.length > 0,
				onSelect: () => openDelete(secret)
			}
		];
	}

	function messageOf(problem: unknown): string {
		if (problem instanceof ApiError) return tErrorCode(problem.code) ?? problem.title;
		return t('table.error');
	}

	// A conflict on delete means a user of the secret appeared after the list was read.
	function isConflict(problem: unknown): boolean {
		const status = (problem as { problem?: { status?: number } } | null)?.problem?.status;
		return status === 409;
	}

	async function load(): Promise<void> {
		loading = true;
		error = null;
		try {
			secrets = await service.listSecrets();
		} catch (problem) {
			error = messageOf(problem);
		} finally {
			loading = false;
		}
	}

	onMount(load);

	function openCreate(): void {
		newName = '';
		newPurpose = 'hook_secret';
		newValue = '';
		createError = null;
		createProblem = null;
		createSubmitted = false;
		createOpen = true;
	}

	async function createSecret(): Promise<void> {
		createSubmitted = true;
		if (
			focusFirstInvalid([
				['secret-name', newName === ''],
				['secret-value', newValue === '']
			])
		)
			return;
		creating = true;
		createError = null;
		createProblem = null;
		try {
			await service.createSecret({ name: newName.trim(), purpose: newPurpose, value: newValue });
			createOpen = false;
			pushToast('success', t('secrets.created'));
			await load();
		} catch (problem) {
			createProblem = problem instanceof ApiError ? problem.problem : null;
			createError = namesField(createProblem, createFields) ? null : messageOf(problem);
		} finally {
			creating = false;
		}
	}

	function openReplace(secret: Secret): void {
		replaceTarget = secret;
		replaceValue = '';
		replaceError = null;
		replaceProblem = null;
		replaceSubmitted = false;
		replaceOpen = true;
	}

	async function confirmReplace(): Promise<void> {
		if (!replaceTarget) return;
		replaceSubmitted = true;
		if (focusFirstInvalid([['secret-replace-value', replaceValue === '']])) return;
		replacing = true;
		replaceError = null;
		replaceProblem = null;
		try {
			await service.replaceSecretValue(replaceTarget.id, replaceValue);
			replaceOpen = false;
			pushToast('success', t('secrets.replaced'));
			await load();
		} catch (problem) {
			replaceProblem = problem instanceof ApiError ? problem.problem : null;
			replaceError = namesField(replaceProblem, ['value']) ? null : messageOf(problem);
		} finally {
			replacing = false;
		}
	}

	function openDelete(secret: Secret): void {
		deleteTarget = secret;
		deleteOpen = true;
	}

	async function confirmDelete(): Promise<void> {
		if (!deleteTarget || deleteTarget.used_by.length > 0) return;
		deleting = true;
		try {
			await service.deleteSecret(deleteTarget.id);
			deleteOpen = false;
			deleteTarget = null;
			pushToast('success', t('secrets.deleted'));
			await load();
		} catch (problem) {
			pushToast('danger', isConflict(problem) ? t('secrets.deleteRefused') : messageOf(problem));
			if (isConflict(problem)) {
				deleteOpen = false;
				await load();
			}
		} finally {
			deleting = false;
		}
	}

	// Drop typed values when a dialog closes.
	$effect(() => {
		if (!createOpen) newValue = '';
	});
	$effect(() => {
		if (!replaceOpen) replaceValue = '';
	});
</script>

{#snippet usersList(secret: Secret)}
	<ul class="users">
		{#each secret.used_by as usage (usage.kind + usage.id)}
			<li>
				<span class="users__kind">{t(USAGE_KEYS[usage.kind])}</span>
				<a href={resolve(usageHref(usage))}>{usage.name}</a>
			</li>
		{/each}
	</ul>
{/snippet}

{#snippet detailUsers()}
	{#if detail}
		{@render usersList(detail)}
	{/if}
{/snippet}

<SettingsLayout
	title={t('secrets.title')}
	description={t('secrets.description')}
	count={loading ? undefined : secrets.length}
>
	{#snippet actions()}
		{#if canWrite}
			<Button variant="primary" icon="plus" onclick={openCreate}>{t('secrets.create')}</Button>
		{/if}
	{/snippet}

	<ListView
		label={t('secrets.title')}
		items={shown}
		itemKey={(secret) => secret.id}
		icon={(secret) => iconOf(secretState(secret))}
		pill={(secret) => pillOf(secretState(secret))}
		title={(secret) => secret.name}
		onOpen={openDetail}
		{meta}
		actions={canWrite ? rowActions : undefined}
		moreLabel={(secret) => t('secrets.moreActions', { name: secret.name })}
		{loading}
		{error}
		{filtered}
		onReset={resetFilters}
		emptyIcon="lock"
		emptyTitle={t('secrets.emptyTitle')}
	>
		{#snippet toolbar()}
			<FilterBar
				label={t('secrets.filters')}
				value={filters.value}
				search={{ key: 'q', label: t('hosts.search'), placeholder: t('secrets.searchPlaceholder') }}
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

	<Alert variant="inline" icon="lock" title={t('secrets.note')} />
</SettingsLayout>

<Dialog bind:open={createOpen} size="md" title={t('secrets.createTitle')} onsubmit={createSecret}>
	{#if createError}
		<Alert variant="inline" tone="danger" title={createError} />
	{/if}
	<TextField
		id="secret-name"
		label={t('secrets.name')}
		help={t('secrets.nameHint')}
		error={emptyError(createSubmitted, newName) ?? fieldError(createProblem, 'name')}
		bind:value={newName}
		placeholder={t('secrets.namePlaceholder')}
		required
	/>
	<Select
		id="secret-purpose"
		label={t('secrets.purpose')}
		error={fieldError(createProblem, 'purpose')}
		bind:value={newPurpose}
		options={purposeOptions}
		required
	/>
	<PasswordField
		id="secret-value"
		label={t('secrets.value')}
		help={t('secrets.valueHint')}
		error={emptyError(createSubmitted, newValue) ?? fieldError(createProblem, 'value')}
		autocomplete="new-password"
		bind:value={newValue}
		required
	/>
	{#snippet footer()}
		<Button onclick={() => (createOpen = false)}>{t('common.cancel')}</Button>
		<Button type="submit" variant="primary" loading={creating}>
			{t('secrets.create')}
		</Button>
	{/snippet}
</Dialog>

<Dialog
	bind:open={replaceOpen}
	size="md"
	title={t('secrets.replaceTitle')}
	onsubmit={confirmReplace}
>
	<Alert tone="attention" icon="lock" title={t('secrets.replaceHidden')}>
		{t('secrets.replaceHiddenText')}
	</Alert>
	{#if replaceError}
		<Alert variant="inline" tone="danger" title={replaceError} />
	{/if}
	<TextField
		id="secret-replace-name"
		label={t('secrets.name')}
		value={replaceTarget?.name ?? ''}
		readonly
	/>
	<PasswordField
		id="secret-replace-value"
		label={t('secrets.newValue')}
		error={emptyError(replaceSubmitted, replaceValue) ?? fieldError(replaceProblem, 'value')}
		autocomplete="new-password"
		bind:value={replaceValue}
		required
	/>
	<p class="dialog-note">{t('secrets.replaceDependents')}</p>
	{#snippet footer()}
		<Button onclick={() => (replaceOpen = false)}>{t('common.cancel')}</Button>
		<Button type="submit" variant="primary" loading={replacing}>
			{t('secrets.replace')}
		</Button>
	{/snippet}
</Dialog>

<Dialog
	bind:open={deleteOpen}
	size="sm"
	icon="octagon"
	title={deleteTarget && deleteTarget.used_by.length > 0
		? t('secrets.deleteInUseTitle')
		: t('secrets.deleteTitle')}
>
	{#if deleteTarget && deleteTarget.used_by.length > 0}
		<p>{t('secrets.deleteInUseBody', { name: deleteTarget.name })}</p>
		{@render usersList(deleteTarget)}
	{:else}
		<p>{t('secrets.deleteBody', { name: deleteTarget?.name ?? '' })}</p>
	{/if}
	{#snippet footer()}
		<Button onclick={() => (deleteOpen = false)}>{t('common.cancel')}</Button>
		<Button
			variant="danger"
			loading={deleting}
			disabled={!deleteTarget || deleteTarget.used_by.length > 0}
			onclick={confirmDelete}
		>
			{t('secrets.delete')}
		</Button>
	{/snippet}
</Dialog>

<style>
	.users {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-1);
		margin: 0;
		padding: 0;
		list-style: none;
	}

	.users__kind {
		color: var(--rh-color-text-subtle);
		margin-right: var(--rh-space-1);
	}

	.dialog-note {
		margin: 0;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}
</style>
