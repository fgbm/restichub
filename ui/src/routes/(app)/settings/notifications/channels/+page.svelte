<script lang="ts">
	import { onMount } from 'svelte';
	import { ApiError, fieldError, namesField, service } from '#lib/api/index.ts';
	import type {
		ApiProblem,
		ChannelKind,
		CreateChannelRequest,
		NotificationChannel,
		UpdateChannelRequest
	} from '#lib/api/index.ts';
	import { SettingsLayout } from '#lib/components/index.ts';
	import {
		Alert,
		Button,
		Checkbox,
		CopyField,
		Dialog,
		DropdownMenu,
		EmptyState,
		Field,
		Select,
		StatusPill,
		Table,
		TextField
	} from '#lib/components/ui/index.ts';
	import type { MenuItem, SelectOption, TableColumn } from '#lib/components/ui/index.ts';
	import { t, tChannelError, tErrorCode } from '#lib/i18n/index.ts';
	import type { MessageKey } from '#lib/i18n/index.ts';
	import { formatTime, relativeTime } from '#lib/format.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';

	const FIELDS = ['name', 'kind', 'recipients', 'url'];

	let channels = $state<NotificationChannel[]>([]);
	let emailConfigured = $state(true);
	let loading = $state(true);
	let error = $state<string | null>(null);

	let formOpen = $state(false);
	let editing = $state<NotificationChannel | null>(null);
	let saving = $state(false);
	let formError = $state<string | null>(null);
	let formProblem = $state<ApiProblem | null>(null);
	let name = $state('');
	let kind = $state<ChannelKind>('email');
	let enabled = $state(true);
	let recipients = $state('');
	let url = $state('');

	let secret = $state<string | null>(null);
	let secretOpen = $state(false);

	let deleteTarget = $state<NotificationChannel | null>(null);
	let deleteOpen = $state(false);
	let deleting = $state(false);

	const kindOptions = $derived<SelectOption[]>([
		{ value: 'email', label: t('channels.kind.email') },
		{ value: 'webhook', label: t('channels.kind.webhook') }
	]);

	const columns = $derived<TableColumn<NotificationChannel>[]>([
		{ key: 'name', header: t('channels.name'), render: nameCell },
		{
			key: 'kind',
			header: t('channels.kind'),
			cell: (row) => t(`channels.kind.${row.kind}` as MessageKey),
			badge: () => 'neutral'
		},
		{ key: 'target', header: t('channels.target'), value: targetOf, render: targetCell },
		{
			key: 'enabled',
			header: t('common.status'),
			value: (row) => (row.enabled ? t('common.enabled') : t('common.disabled')),
			render: stateCell
		},
		{ key: 'created_at', header: t('common.createdAt'), nowrap: true, render: createdCell }
	]);

	function targetOf(channel: NotificationChannel): string {
		return channel.kind === 'email' ? channel.recipients.join(', ') : (channel.url_origin ?? '');
	}

	function messageOf(problem: unknown): string {
		if (problem instanceof ApiError) return tErrorCode(problem.code) ?? problem.title;
		return t('table.error');
	}

	// The mock throws a plain error that holds the problem, the way the plans screen reads it.
	function problemOf(problem: unknown): ApiProblem | null {
		if (problem instanceof ApiError) return problem.problem;
		if (problem && typeof problem === 'object' && 'problem' in problem) {
			return (problem as { problem: ApiProblem }).problem;
		}
		return null;
	}

	// The reason of a URL problem says more than the generic text: the scheme and the address
	// each have a message of their own.
	const urlError = $derived.by(() => {
		if (formProblem?.field !== 'url') return undefined;
		if (formProblem.reason === 'unsupported') return t('channels.url.scheme');
		if (formProblem.reason === 'reserved') return t('channels.url.blocked');
		return fieldError(formProblem, 'url');
	});

	async function load(): Promise<void> {
		loading = true;
		error = null;
		try {
			const list = await service.listChannels();
			channels = list.items;
			emailConfigured = list.email_configured;
		} catch (problem) {
			error = messageOf(problem);
		} finally {
			loading = false;
		}
	}

	onMount(load);

	function resetForm(): void {
		name = '';
		kind = 'email';
		enabled = true;
		recipients = '';
		url = '';
		formError = null;
		formProblem = null;
	}

	function openCreate(): void {
		editing = null;
		resetForm();
		formOpen = true;
	}

	function openEdit(channel: NotificationChannel): void {
		editing = channel;
		resetForm();
		name = channel.name;
		kind = channel.kind;
		enabled = channel.enabled;
		recipients = channel.recipients.join('\n');
		formOpen = true;
	}

	function recipientList(): string[] {
		return recipients
			.split(/[\n,;]+/)
			.map((address) => address.trim())
			.filter((address) => address !== '');
	}

	function showSecret(value: string | null | undefined): void {
		if (!value) return;
		secret = value;
		secretOpen = true;
	}

	// The secret shows once. Clear it whenever the dialog closes.
	$effect(() => {
		if (!secretOpen) secret = null;
	});

	async function save(): Promise<void> {
		saving = true;
		formError = null;
		formProblem = null;
		try {
			let saved;
			if (editing) {
				const patch: UpdateChannelRequest = { name, enabled };
				if (editing.kind === 'email') patch.recipients = recipientList();
				else if (url.trim()) patch.url = url.trim();
				saved = await service.updateChannel(editing.id, patch);
			} else {
				const request: CreateChannelRequest =
					kind === 'email'
						? { name, kind, enabled, recipients: recipientList() }
						: { name, kind, enabled, url: url.trim() };
				saved = await service.createChannel(request);
			}
			pushToast('success', t(editing ? 'channels.updated' : 'channels.created'));
			formOpen = false;
			resetForm();
			showSecret(saved.signing_secret);
			await load();
		} catch (problem) {
			formProblem = problemOf(problem);
			formError = namesField(formProblem, FIELDS) ? null : messageOf(problem);
		} finally {
			saving = false;
		}
	}

	async function rotate(channel: NotificationChannel): Promise<void> {
		try {
			const saved = await service.updateChannel(channel.id, { rotate_secret: true });
			formOpen = false;
			showSecret(saved.signing_secret);
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		}
	}

	async function sendTest(channel: NotificationChannel): Promise<void> {
		try {
			const result = await service.testChannel(channel.id);
			if (result.ok) {
				pushToast('success', t('channels.testSent'));
			} else {
				const code = result.error ?? 'unknown';
				pushToast('danger', t('channels.testFailed', { error: tChannelError(code) ?? code }));
			}
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		}
	}

	async function confirmDelete(): Promise<void> {
		if (!deleteTarget) return;
		deleting = true;
		try {
			await service.deleteChannel(deleteTarget.id);
			pushToast('success', t('channels.deleted'));
			deleteOpen = false;
			deleteTarget = null;
			await load();
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		} finally {
			deleting = false;
		}
	}

	function actionsFor(channel: NotificationChannel): MenuItem[] {
		return [
			{ label: t('channels.test'), icon: 'mail', onSelect: () => void sendTest(channel) },
			{ label: t('common.edit'), icon: 'settings', onSelect: () => openEdit(channel) },
			{
				label: t('channels.delete'),
				icon: 'octagon',
				tone: 'danger',
				onSelect: () => {
					deleteTarget = channel;
					deleteOpen = true;
				}
			}
		];
	}

	const empty = $derived(!loading && !error && channels.length === 0);
</script>

{#snippet nameCell(channel: NotificationChannel)}
	<span class="channel__name">{channel.name}</span>
{/snippet}

{#snippet targetCell(channel: NotificationChannel)}
	<span class="channel__target">{targetOf(channel)}</span>
{/snippet}

{#snippet stateCell(channel: NotificationChannel)}
	<StatusPill tone={channel.enabled ? 'success' : 'neutral'}>
		{channel.enabled ? t('common.enabled') : t('common.disabled')}
	</StatusPill>
{/snippet}

{#snippet createdCell(channel: NotificationChannel)}
	<time datetime={channel.created_at} title={formatTime(channel.created_at)}>
		{relativeTime(channel.created_at)}
	</time>
{/snippet}

<svelte:head>
	<title>{t('channels.title')} · {t('app.name')}</title>
</svelte:head>

<SettingsLayout
	title={t('channels.title')}
	description={t('channels.description')}
	count={loading ? undefined : channels.length}
>
	{#snippet actions()}
		<Button variant="primary" icon="plus" onclick={openCreate}>{t('channels.add')}</Button>
	{/snippet}

	{#if !emailConfigured}
		<Alert tone="attention" icon="mail" title={t('channels.emailNotConfigured')} />
	{/if}

	{#if empty}
		<EmptyState
			icon="mail"
			title={t('channels.emptyTitle')}
			description={t('channels.emptyDescription')}
			bordered
		>
			<Button variant="primary" icon="plus" onclick={openCreate}>{t('channels.addFirst')}</Button>
		</EmptyState>
	{:else}
		<Table
			{columns}
			rows={channels}
			{loading}
			{error}
			rowKey={(channel) => channel.id}
			filterPlaceholder={t('channels.searchPlaceholder')}
		>
			{#snippet rowActions(channel)}
				<DropdownMenu
					icon="more"
					align="end"
					label={t('channels.moreActions', { name: channel.name })}
					items={actionsFor(channel)}
				/>
			{/snippet}
		</Table>
	{/if}
</SettingsLayout>

<Dialog
	bind:open={formOpen}
	size="lg"
	title={editing ? t('channels.editTitle') : t('channels.addTitle')}
	onsubmit={save}
>
	{#if formError}
		<Alert variant="inline" tone="danger" title={formError} />
	{/if}
	<TextField
		id="channel-name"
		label={t('channels.name')}
		placeholder={t('channels.namePlaceholder')}
		error={fieldError(formProblem, 'name')}
		bind:value={name}
		required
	/>
	<Select
		id="channel-kind"
		label={t('channels.kind')}
		options={kindOptions}
		disabled={!!editing}
		bind:value={kind}
	/>
	{#if kind === 'email'}
		<Field
			label={t('channels.recipients')}
			help={t('channels.recipientsHint')}
			error={fieldError(formProblem, 'recipients')}
			for="channel-recipients"
		>
			<textarea
				id="channel-recipients"
				class="rh-textarea rh-input--mono"
				rows="3"
				placeholder="ops@example.com"
				bind:value={recipients}></textarea>
		</Field>
	{:else}
		<TextField
			id="channel-url"
			label={t('channels.url')}
			help={editing ? t('channels.urlEditHint') : t('channels.urlHint')}
			error={urlError}
			placeholder="https://hooks.example.com/…"
			bind:value={url}
			required={!editing}
		/>
		{#if editing}
			<div>
				<Button type="button" icon="key" onclick={() => editing && void rotate(editing)}>
					{t('channels.rotate')}
				</Button>
			</div>
		{/if}
	{/if}
	<Checkbox label={t('channels.enabled')} bind:checked={enabled} />
	{#snippet footer()}
		<Button onclick={() => (formOpen = false)}>{t('common.cancel')}</Button>
		<Button type="submit" variant="primary" loading={saving} disabled={!name}>
			{t('common.save')}
		</Button>
	{/snippet}
</Dialog>

<Dialog
	bind:open={secretOpen}
	size="lg"
	title={t('channels.secretTitle')}
	onsubmit={() => (secretOpen = false)}
>
	<Alert tone="attention" icon="key" title={t('channels.secretBody')}>
		{t('channels.secretText')}
	</Alert>
	{#if secret}
		<CopyField value={secret} label={t('channels.secretValue')} secret />
	{/if}
	{#snippet footer()}
		<Button type="submit" variant="primary">{t('common.done')}</Button>
	{/snippet}
</Dialog>

<Dialog bind:open={deleteOpen} size="sm" icon="octagon" title={t('channels.deleteTitle')}>
	<p>{t('channels.deleteBody', { name: deleteTarget?.name ?? '' })}</p>
	{#snippet footer()}
		<Button onclick={() => (deleteOpen = false)}>{t('common.cancel')}</Button>
		<Button variant="danger" loading={deleting} onclick={confirmDelete}>
			{t('channels.delete')}
		</Button>
	{/snippet}
</Dialog>

<style>
	.channel__name {
		font-weight: var(--rh-font-weight-semibold);
	}

	.channel__target {
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
		word-break: break-word;
	}
</style>
