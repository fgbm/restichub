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
		PasswordField,
		Select,
		Table,
		TextField,
		RowActions
	} from '#lib/components/ui/index.ts';
	import type { SelectOption, TableColumn } from '#lib/components/ui/index.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import { formatTime, relativeTime } from '#lib/format.ts';
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
	const createFields = ['name', 'purpose', 'value'];

	let replaceTarget = $state<Secret | null>(null);
	let replaceOpen = $state(false);
	let replacing = $state(false);
	let replaceError = $state<string | null>(null);
	let replaceProblem = $state<ApiProblem | null>(null);
	let replaceValue = $state('');

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

	const columns = $derived<TableColumn<Secret>[]>([
		{ key: 'name', header: t('secrets.name'), nowrap: true, render: nameCell },
		{
			key: 'purpose',
			header: t('secrets.purpose'),
			cell: (row) => purposeLabel(row.purpose)
		},
		{
			key: 'used_by',
			header: t('secrets.usedBy'),
			value: (row) => row.used_by.map((usage) => usage.name).join(' '),
			render: usedByCell
		},
		{ key: 'created_at', header: t('common.createdAt'), nowrap: true, render: createdCell },
		{
			key: 'updated_at',
			header: t('secrets.replacedAt'),
			nowrap: true,
			render: replacedCell
		}
	]);

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
		createOpen = true;
	}

	async function createSecret(): Promise<void> {
		if (!newName || !newValue) return;
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
		replaceOpen = true;
	}

	async function confirmReplace(): Promise<void> {
		if (!replaceTarget || !replaceValue) return;
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

{#snippet rowActions(secret: Secret)}
	<RowActions
		moreLabel={t('secrets.moreActions', { name: secret.name })}
		items={[
			...(canReplaceValue(secret)
				? [
						{
							label: t('secrets.replace'),
							icon: 'refresh' as const,
							onSelect: () => openReplace(secret)
						}
					]
				: []),
			{
				label: t('secrets.delete'),
				icon: 'trash',
				tone: 'danger',
				onSelect: () => openDelete(secret)
			}
		]}
	/>
{/snippet}

{#snippet nameCell(secret: Secret)}
	<span class="secret-name" title={secret.name}>{secret.name}</span>
{/snippet}

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

{#snippet usedByCell(secret: Secret)}
	{#if secret.used_by.length === 0}
		<span class="muted">{t('secrets.unused')}</span>
	{:else}
		{@render usersList(secret)}
	{/if}
{/snippet}

{#snippet createdCell(secret: Secret)}
	<time datetime={secret.created_at} title={formatTime(secret.created_at)}>
		{relativeTime(secret.created_at)}
	</time>
{/snippet}

{#snippet replacedCell(secret: Secret)}
	{#if wasReplaced(secret)}
		<time datetime={secret.updated_at} title={formatTime(secret.updated_at)}>
			{relativeTime(secret.updated_at)}
		</time>
	{:else}
		<span class="muted">{t('common.never')}</span>
	{/if}
{/snippet}

<svelte:head>
	<title>{t('secrets.title')} · {t('app.name')}</title>
</svelte:head>

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

	<Table
		{columns}
		rows={secrets}
		{loading}
		{error}
		rowKey={(secret) => secret.id}
		rowActions={canWrite ? rowActions : undefined}
		filterPlaceholder={t('secrets.searchPlaceholder')}
	/>

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
		error={fieldError(createProblem, 'name')}
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
		error={fieldError(createProblem, 'value')}
		autocomplete="new-password"
		bind:value={newValue}
		required
	/>
	{#snippet footer()}
		<Button onclick={() => (createOpen = false)}>{t('common.cancel')}</Button>
		<Button type="submit" variant="primary" loading={creating} disabled={!newName || !newValue}>
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
		error={fieldError(replaceProblem, 'value')}
		autocomplete="new-password"
		bind:value={replaceValue}
		required
	/>
	<p class="dialog-note">{t('secrets.replaceDependents')}</p>
	{#snippet footer()}
		<Button onclick={() => (replaceOpen = false)}>{t('common.cancel')}</Button>
		<Button type="submit" variant="primary" loading={replacing} disabled={!replaceValue}>
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
	.secret-name {
		display: block;
		max-width: 28ch;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		font-weight: var(--rh-font-weight-semibold);
	}

	.muted {
		color: var(--rh-color-text-subtle);
	}

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
