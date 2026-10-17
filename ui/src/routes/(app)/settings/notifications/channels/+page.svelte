<script lang="ts">
	import { resolve } from '$app/paths';
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
		FilterBar,
		ListView,
		Select,
		TagInput,
		TextField
	} from '#lib/components/ui/index.ts';
	import type { FilterFacet, ListMeta, RowAction, SelectOption } from '#lib/components/ui/index.ts';
	import { ago, changedAt } from '#lib/list-format.ts';
	import { channelState, iconOf, pillOf } from '#lib/vocabulary.ts';
	import { facetCounts, matchesSearch, splitList } from '#lib/filters.ts';
	import { queryFilters } from '#lib/stores/query-filters.svelte.ts';
	import { t, tChannelError, tErrorCode } from '#lib/i18n/index.ts';
	import type { MessageKey } from '#lib/i18n/index.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';

	const FIELDS = ['name', 'kind', 'recipients', 'url'];
	// The server takes at most 20 recipients and keeps these characters out of the SMTP envelope.
	const MAX_RECIPIENTS = 20;
	const ADDRESS = /^[^\s@<>,;"]+@[^\s@<>,;"]+$/;

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
	let recipients = $state<string[]>([]);
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

	const filters = queryFilters(['q', 'kind', 'state', 'sort', 'order'] as const);

	const facets = $derived<FilterFacet[]>([
		{
			key: 'kind',
			label: t('channels.kind'),
			options: facetCounts(channels, (channel) => [channel.kind]).map((option) => ({
				...option,
				label: t(`channels.kind.${option.value}` as MessageKey)
			}))
		}
	]);

	const matching = $derived(
		channels.filter((channel) => {
			const { q, kind } = filters.value;
			if (kind && !splitList(kind).includes(channel.kind)) return false;
			return matchesSearch(q, [channel.name, targetOf(channel)]);
		})
	);

	const states = $derived({
		default: 'all',
		options: [
			{ value: 'all', label: t('channels.tab.all'), count: matching.length },
			{
				value: 'disabled',
				label: t('channels.tab.disabled'),
				count: matching.filter((channel) => !channel.enabled).length
			}
		]
	});

	const sort = $derived({
		default: 'name',
		options: [{ value: 'name', label: t('channels.sort.name') }]
	});

	const shown = $derived.by(() => {
		const list = matching.filter(
			(channel) => filters.value.state !== 'disabled' || !channel.enabled
		);
		const factor = filters.value.order === 'desc' ? -1 : 1;
		return [...list].sort((a, b) => factor * a.name.localeCompare(b.name));
	});

	const filtered = $derived(
		channels.length > 0 && Boolean(filters.value.q || filters.value.kind || filters.value.state)
	);

	function resetFilters(): void {
		filters.update({ q: '', kind: '', state: '' });
	}

	function meta(channel: NotificationChannel): ListMeta[] {
		const target =
			channel.kind === 'email'
				? channel.recipients.length > 1
					? `${channel.recipients[0]} +${channel.recipients.length - 1}`
					: (channel.recipients[0] ?? '')
				: { text: channel.url_origin ?? '', mono: true };
		return [
			t(`channels.kind.${channel.kind}` as MessageKey),
			target,
			channel.last_delivery_at && {
				text: t(
					channel.last_delivery_ok ? 'channels.meta.delivered' : 'channels.meta.deliveryFailed',
					{
						time: ago(channel.last_delivery_at)
					}
				),
				time: channel.last_delivery_at
			},
			{
				text: t('list.changed', { time: changedAt(channel.updated_at) }),
				time: channel.updated_at
			}
		];
	}

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
		recipients = [];
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
		recipients = [...channel.recipients];
		formOpen = true;
	}

	function checkAddress(address: string): string | null {
		return ADDRESS.test(address) ? null : t('channels.recipientInvalid');
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
				if (editing.kind === 'email') patch.recipients = [...recipients];
				else if (url.trim()) patch.url = url.trim();
				saved = await service.updateChannel(editing.id, patch);
			} else {
				const request: CreateChannelRequest =
					kind === 'email'
						? { name, kind, enabled, recipients: [...recipients] }
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

	async function toggle(channel: NotificationChannel): Promise<void> {
		try {
			await service.updateChannel(channel.id, { enabled: !channel.enabled });
			pushToast('success', t(channel.enabled ? 'channels.disabledToast' : 'channels.enabledToast'));
			await load();
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		}
	}

	function openDelete(): void {
		if (!editing) return;
		deleteTarget = editing;
		formOpen = false;
		deleteOpen = true;
	}

	// Delete is in the channel form, so an item keeps two actions.
	function actionsFor(channel: NotificationChannel): RowAction[] {
		return [
			{
				id: 'test',
				label: t('channels.test'),
				icon: 'send',
				disabled: !channel.enabled,
				onSelect: () => void sendTest(channel)
			},
			{
				id: 'toggle',
				label: t(channel.enabled ? 'channels.disable' : 'channels.enable'),
				icon: 'power',
				onSelect: () => void toggle(channel)
			}
		];
	}
</script>

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

	<!-- A channel has no record page: the name opens its form (design/style/ui-principles.md, 2.4). -->
	<ListView
		label={t('channels.title')}
		items={shown}
		itemKey={(channel) => channel.id}
		icon={(channel) => iconOf(channelState(channel))}
		pill={(channel) => pillOf(channelState(channel))}
		title={(channel) => channel.name}
		onOpen={openEdit}
		{meta}
		actions={actionsFor}
		moreLabel={(channel) => t('channels.moreActions', { name: channel.name })}
		{loading}
		{error}
		{filtered}
		onReset={resetFilters}
		emptyIcon="mail"
		emptyTitle={t('channels.emptyTitle')}
		emptyDescription={t('channels.emptyDescription')}
	>
		{#snippet emptyActions()}
			<Button variant="primary" onclick={openCreate}>{t('channels.addFirst')}</Button>
		{/snippet}
		{#snippet toolbar()}
			<FilterBar
				label={t('channels.filters')}
				value={filters.value}
				search={{
					key: 'q',
					label: t('hosts.search'),
					placeholder: t('channels.searchPlaceholder')
				}}
				{states}
				{facets}
				{sort}
				onchange={filters.update}
				onreset={resetFilters}
			/>
		{/snippet}
	</ListView>

	{#if channels.length > 0}
		<a
			class="manage-secrets"
			href={resolve('/(app)/settings/access/secrets')}
			target="_blank"
			rel="noopener"
		>
			{t('secrets.manage')}
		</a>
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
		<TagInput
			id="channel-recipients"
			label={t('channels.recipients')}
			help={t('channels.recipientsHint')}
			error={fieldError(formProblem, 'recipients')}
			placeholder={t('common.example', { value: 'ops@example.com' })}
			mono
			max={MAX_RECIPIENTS}
			separator={/[\s,;]+/}
			check={checkAddress}
			bind:value={recipients}
		/>
	{:else}
		<TextField
			id="channel-url"
			label={t('channels.url')}
			help={editing ? t('channels.urlEditHint') : t('channels.urlHint')}
			error={urlError}
			placeholder={t('common.example', { value: 'https://hooks.example.com/…' })}
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
		{#if editing}
			<span class="footer__start">
				<Button variant="danger" icon="trash" onclick={openDelete}>{t('channels.delete')}</Button>
			</span>
		{/if}
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
	.footer__start {
		margin-right: auto;
	}

	.manage-secrets {
		align-self: flex-start;
		font-size: var(--rh-font-size-sm);
	}
</style>
