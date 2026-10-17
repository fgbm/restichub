<script lang="ts">
	import { onMount } from 'svelte';
	import { resolve } from '$app/paths';
	import HostActions from '#lib/components/HostActions.svelte';
	import { eventReader } from '#lib/events.ts';
	import { isStreamConnected, onEvent } from '#lib/stores/events.svelte.ts';
	import { ApiError, fieldError, service } from '#lib/api/index.ts';
	import type {
		Agent,
		AgentListFacets,
		AgentQuery,
		EnrollmentToken,
		EnrollmentTokenCreated,
		ApiProblem
	} from '#lib/api/index.ts';
	import { Page } from '#lib/components/layout/index.ts';
	import {
		Alert,
		Button,
		ConfirmDialog,
		CopyField,
		Dialog,
		FilterBar,
		ListView,
		Panel,
		TagInput
	} from '#lib/components/ui/index.ts';
	import type { FilterFacet, ListMeta, RowAction } from '#lib/components/ui/index.ts';
	import { queryFilters } from '#lib/stores/query-filters.svelte.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import { formatCount } from '#lib/format.ts';
	import { ago, inFuture } from '#lib/list-format.ts';
	import { ENTITY_ICONS, hostState, iconOf, pillOf } from '#lib/vocabulary.ts';
	import { PAGE_SIZES, lastPageIfPast, pageNumber, pageSizeOf } from '#lib/list-page.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';

	let hostActions: HostActions;
	let enrollmentTokens = $state<EnrollmentToken[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);

	let addOpen = $state(false);
	let creating = $state(false);
	let createError = $state<string | null>(null);
	let createProblem = $state<ApiProblem | null>(null);
	let labels = $state<string[]>([]);
	let createdToken = $state<EnrollmentTokenCreated | null>(null);

	let cancelTarget = $state<EnrollmentToken | null>(null);
	let cancelOpen = $state(false);
	let cancelling = $state(false);

	const canWrite = $derived(getSession()?.role === 'admin');
	// The registration command carries this installation URL (ADR-0032).
	let origin = $state('');
	onMount(() => {
		origin = window.location.origin;
	});
	const installCommand = $derived(
		t('hosts.installPlaceholder', { url: origin, token: createdToken?.token ?? '<token>' })
	);

	// The names of the hosts that a token names. The page rows fill it; a host on another page is
	// read once by its identifier, so the screen never reads the whole host list.
	let hostNames = $state<Record<string, string>>({});
	const askedHosts: Record<string, boolean> = {};

	function rememberHosts(list: Agent[]): void {
		const added = list.filter((agent) => hostNames[agent.id] !== agent.hostname);
		if (added.length === 0) return;
		hostNames = {
			...hostNames,
			...Object.fromEntries(added.map((agent) => [agent.id, agent.hostname]))
		};
	}

	async function resolveHosts(ids: (string | null | undefined)[]): Promise<void> {
		const missing = ids.filter((id): id is string => !!id && !(id in hostNames) && !askedHosts[id]);
		await Promise.all(
			missing.map(async (id) => {
				askedHosts[id] = true;
				try {
					rememberHosts([await service.getAgent(id)]);
				} catch {
					delete askedHosts[id];
				}
			})
		);
	}

	// The token of the open dialog after it left the pending list.
	let createdUse = $state<EnrollmentToken | null>(null);

	// The dialog shows the host when the agent used the token that the dialog created.
	const registeredHost = $derived.by(() => {
		const used = createdUse;
		if (!createdToken || used?.id !== createdToken.id) return null;
		if (!used.used_at || !used.used_by_agent_id) return null;
		const hostname = hostNames[used.used_by_agent_id];
		return hostname ? { hostname } : null;
	});

	function replacesName(token: EnrollmentToken): string | null {
		if (!token.replaces_agent_id) return null;
		return hostNames[token.replaces_agent_id] ?? null;
	}

	// A token stays in the pending list only while it can still enroll a host (FR-ID-01).
	const pending = $derived(
		enrollmentTokens.filter(
			(token) =>
				!token.used_at && !token.revoked_at && new Date(token.expires_at).getTime() > Date.now()
		)
	);

	// The list filters, sorts and pages on the server (FR-UI-01). Without a state in the address the
	// list leaves out the revoked hosts; the state facet shows how many there are.
	const VISIBLE_STATES = 'online,offline,waiting';
	const filters = queryFilters([
		'q',
		'state',
		'os',
		'agent_version',
		'label',
		'scripts',
		'page',
		'size',
		'sort',
		'order'
	] as const);

	let pageRows = $state<Agent[]>([]);
	let total = $state(0);
	let totalCapped = $state(false);
	let facetValues = $state<AgentListFacets | null>(null);
	let pageLoading = $state(true);
	let pageError = $state<string | null>(null);

	const SORT_KEYS: Record<string, NonNullable<AgentQuery['sort']>> = {
		hostname: 'hostname',
		last_seen_at: 'last_seen_at',
		agent_version: 'agent_version'
	};
	const sort = $derived({
		default: 'hostname',
		options: [
			{ value: 'hostname', label: t('hosts.sort.name') },
			{ value: 'last_seen_at', label: t('hosts.sort.lastSeen') },
			{ value: 'agent_version', label: t('hosts.sort.agent') }
		]
	});
	const tablePage = $derived(pageNumber(filters.value.page));
	const pageSize = $derived(pageSizeOf(filters.value.size));

	function queryOf(): AgentQuery {
		const value = filters.value;
		return {
			q: value.q.trim() || undefined,
			state: value.state || VISIBLE_STATES,
			os: value.os || undefined,
			agent_version: value.agent_version || undefined,
			label: value.label || undefined,
			server_scripts:
				value.scripts === 'enabled' || value.scripts === 'disabled' ? value.scripts : undefined,
			sort: SORT_KEYS[value.sort || 'hostname'],
			order: value.order === 'desc' ? 'desc' : 'asc',
			offset: (tablePage - 1) * pageSize,
			limit: pageSize
		};
	}

	// The facets follow the filters only, so a new page, a new order and a poll do not count again.
	function filterKey(): string {
		const value = filters.value;
		return JSON.stringify([
			value.q.trim(),
			value.state,
			value.os,
			value.agent_version,
			value.label,
			value.scripts
		]);
	}
	let facetKey: string | null = null;

	// A poll and a filter change can overlap; only the last request may fill the table.
	let request = 0;

	async function loadPage(refreshFacets = false): Promise<void> {
		const mine = ++request;
		const key = filterKey();
		const withFacets = refreshFacets || key !== facetKey;
		try {
			const result = await service.searchAgents({
				...queryOf(),
				facets: withFacets || undefined
			});
			if (mine !== request) return;
			const last = lastPageIfPast(tablePage, pageSize, result.total, result.items.length);
			if (last !== null) {
				filters.update({ page: String(last) });
				return;
			}
			pageRows = result.items;
			total = result.total;
			totalCapped = result.total_capped;
			if (result.facets) {
				facetValues = result.facets;
				facetKey = key;
			}
			rememberHosts(result.items);
			pageError = null;
		} catch (problem) {
			if (mine !== request) return;
			pageError = messageOf(problem);
		} finally {
			if (mine === request) pageLoading = false;
		}
	}

	// The first page loads with the rest of the screen. A typed search waits for a short pause.
	let loadedQuery = JSON.stringify(queryOf());
	let typing: ReturnType<typeof setTimeout> | undefined;
	$effect(() => {
		const key = JSON.stringify(queryOf());
		if (key === loadedQuery) return;
		loadedQuery = key;
		clearTimeout(typing);
		typing = setTimeout(() => void loadPage(), 250);
	});
	onMount(() => () => clearTimeout(typing));

	const facets = $derived<FilterFacet[]>([
		{
			key: 'os',
			label: t('hosts.facet.os'),
			options: (facetValues?.os ?? []).map(({ value, count }) => ({ value, count }))
		},
		{
			key: 'label',
			label: t('hosts.facet.label'),
			options: (facetValues?.labels ?? []).map(({ value, count }) => ({ value, count }))
		},
		{
			key: 'agent_version',
			label: t('hosts.facet.agent'),
			options: (facetValues?.agent_versions ?? []).map(({ value, count }) => ({ value, count }))
		},
		{
			key: 'scripts',
			label: t('hosts.serverScripts'),
			multiple: false,
			options: (['enabled', 'disabled'] as const).map((value) => ({
				value,
				label: t(`hosts.serverScripts.${value}`)
			}))
		}
	]);

	const states = $derived({
		default: 'active',
		options: [
			{
				value: 'active',
				label: t('hosts.tab.active'),
				count: facetValues
					? facetValues.states.online + facetValues.states.offline + facetValues.states.waiting
					: undefined
			},
			{
				value: 'offline',
				label: t('hosts.tab.offline'),
				count: facetValues?.states.offline
			},
			{ value: 'revoked', label: t('hosts.tab.revoked'), count: facetValues?.states.revoked }
		]
	});

	function resetFilters(): void {
		filters.update({ q: '', state: '', os: '', agent_version: '', label: '', scripts: '' });
	}

	function contact(agent: Agent): ListMeta {
		if (!agent.first_connected_at) {
			return {
				text: t('hosts.meta.registered', { time: ago(agent.created_at) }),
				time: agent.created_at
			};
		}
		if (!agent.last_seen_at) return t('hosts.meta.neverSeen');
		return {
			text: t('hosts.meta.contact', { time: ago(agent.last_seen_at) }),
			time: agent.last_seen_at
		};
	}

	function labelsText(agent: Agent): string {
		if (agent.labels.length === 0) return '';
		const first = agent.labels.slice(0, 2).join(', ');
		return agent.labels.length > 2 ? `${first} +${agent.labels.length - 2}` : first;
	}

	function meta(agent: Agent): ListMeta[] {
		const update = agent.version_status === 'update_available' && agent.status === 'active';
		return [
			contact(agent),
			`${agent.os} · ${agent.arch}`,
			update
				? t('hosts.meta.agentUpdate', {
						version: agent.agent_version,
						latest: agent.control_plane_version
					})
				: t('hosts.meta.agent', { version: agent.agent_version }),
			labelsText(agent)
		];
	}

	function tokenMeta(token: EnrollmentToken): ListMeta[] {
		return [
			token.labels.length > 0
				? t('hosts.pending.labels', { labels: token.labels.join(', ') })
				: null,
			token.replaces_agent_id
				? t('hosts.pending.reregisters', {
						name: replacesName(token) ?? token.replaces_agent_id
					})
				: null,
			t('hosts.pending.createdBy', { name: createdByLabel(token) }),
			{
				text: t('time.expiresIn', { time: inFuture(token.expires_at) }),
				time: token.expires_at
			}
		];
	}

	function rowActions(agent: Agent): RowAction[] {
		if (!canWrite || agent.status !== 'active') return [];
		return [
			{
				id: 'rename',
				label: t('hosts.rename'),
				icon: 'tag',
				onSelect: () => hostActions.openRename(agent)
			},
			{
				id: 'reregister',
				label: t('hosts.reregister'),
				icon: 'key',
				onSelect: () => hostActions.openReregister(agent)
			},
			{
				id: 'replace',
				label: t('hosts.replaceOld'),
				icon: 'host',
				hidden: !agent.possible_duplicate_of?.is_older,
				onSelect: () => hostActions.openReplace(agent)
			},
			{
				id: 'revoke',
				label: t('hosts.revokeAgent'),
				icon: 'ban',
				danger: true,
				onSelect: () => hostActions.openRevoke(agent)
			}
		];
	}

	// The token row shows the creator's name when the caller created it (getSession is the caller).
	function createdByLabel(token: EnrollmentToken): string {
		const user = getSession()?.user;
		return user && user.id === token.created_by ? user.display_name : token.created_by;
	}

	// The state counts ignore the state filter, so without another filter they count every host.
	const narrowed = $derived(
		Boolean(
			filters.value.q.trim() ||
			filters.value.os ||
			filters.value.agent_version ||
			filters.value.label ||
			filters.value.scripts
		)
	);
	const hostCount = $derived(
		facetValues ? Object.values(facetValues.states).reduce((sum, count) => sum + count, 0) : null
	);
	const firstRun = $derived(!narrowed && hostCount === 0);

	function messageOf(problem: unknown): string {
		if (problem instanceof ApiError) {
			// The server pages only through the rows that it counts.
			if (problem.problem.field === 'offset') return t('filter.tooDeep');
			return tErrorCode(problem.code) ?? problem.title;
		}
		return t('table.error');
	}

	function tokenHosts(): (string | null | undefined)[] {
		return [
			createdUse?.used_by_agent_id,
			...enrollmentTokens.map((token) => token.replaces_agent_id)
		];
	}

	// The used and revoked tokens grow without bound, so the poll reads only the pending ones. The
	// token of the open dialog is read by itself once, when it leaves that list.
	async function readTokens(): Promise<void> {
		enrollmentTokens = await service.listEnrollmentTokens({ pending: true });
		const id = createdToken?.id;
		if (!id || createdUse?.id === id || enrollmentTokens.some((token) => token.id === id)) return;
		const token = await service.getEnrollmentToken(id);
		if (createdToken?.id === id) createdUse = token;
	}

	async function load(): Promise<void> {
		loading = true;
		error = null;
		const page = loadPage(true);
		try {
			await readTokens();
		} catch (problem) {
			error = messageOf(problem);
		}
		await page;
		loading = false;
		reader.loaded();
		await resolveHosts(tokenHosts());
	}

	// A used token leaves the pending list and a new host appears without a manual reload.
	const REFRESH_MS = 10_000;

	async function refresh(): Promise<void> {
		const page = loadPage();
		try {
			await readTokens();
		} catch {
			// The next tick tries again; the visible list stays.
		}
		await page;
		await resolveHosts(tokenHosts());
	}

	// The stream pushes a change of a host. The poll covers a stream that is down.
	const STREAM_REFRESH_MS = 60_000;

	$effect(() => {
		const timer = setInterval(
			() => {
				if (!document.hidden && !loading) void refresh();
			},
			isStreamConnected() ? STREAM_REFRESH_MS : REFRESH_MS
		);
		return () => clearInterval(timer);
	});

	const reader = eventReader(() => loading, refresh, 300);
	// The subscription comes before the first load, so an event during the load is not lost.
	onMount(() => {
		const off = onEvent((event) => {
			if (event.type.startsWith('host.') || event.type === 'resync') reader.notify();
		});
		void load();
		return off;
	});

	// The server takes a label of 1 to 63 characters from a-z, 0-9 and the hyphen.
	const MAX_LABEL = 63;
	function checkLabel(label: string): string | null {
		return /^[a-z0-9-]+$/.test(label) ? null : t('hosts.labelInvalid');
	}

	function openAdd(): void {
		labels = [];
		createError = null;
		createProblem = null;
		createdToken = null;
		addOpen = true;
	}

	async function createToken(): Promise<void> {
		creating = true;
		createError = null;
		createProblem = null;
		try {
			createdToken = await service.createEnrollmentToken({
				labels: labels.length > 0 ? labels : undefined
			});
			pushToast('success', t('hosts.tokenCreated'));
			// The new token shows in the pending list behind the dialog.
			await load();
		} catch (problem) {
			createProblem = problem instanceof ApiError ? problem.problem : null;
			createError = fieldError(createProblem, 'labels') ? null : messageOf(problem);
		} finally {
			creating = false;
		}
	}

	function closeAdd(): void {
		addOpen = false;
		createdToken = null;
	}

	// The token value is shown once. Clear it whenever the dialog closes, also
	// with the Escape key or a click on the backdrop.
	$effect(() => {
		if (!addOpen) createdToken = null;
	});

	function openCancel(token: EnrollmentToken): void {
		cancelTarget = token;
		cancelOpen = true;
	}

	async function confirmCancel(): Promise<void> {
		if (!cancelTarget) return;
		cancelling = true;
		try {
			await service.revokeEnrollmentToken(cancelTarget.id);
			pushToast('success', t('hosts.pending.cancelled'));
			cancelOpen = false;
			cancelTarget = null;
			await load();
		} catch (problem) {
			pushToast('danger', messageOf(problem));
			// The token changed state since the list loaded, for example a host used it.
			cancelOpen = false;
			cancelTarget = null;
			await load();
		} finally {
			cancelling = false;
		}
	}
</script>

<Page
	title={t('hosts.title')}
	description={t('hosts.description')}
	count={pageLoading ? undefined : totalCapped ? `${formatCount(total)}+` : total}
>
	{#snippet actions()}
		{#if canWrite}
			<Button variant="primary" icon="plus" onclick={openAdd}>{t('hosts.add')}</Button>
		{/if}
	{/snippet}

	{#if pending.length > 0}
		<div class="pending">
			<Panel flush>
				{#snippet header()}
					<h2 class="pending__title">
						{t('hosts.pending.title')} <span class="pending__count">{pending.length}</span>
					</h2>
				{/snippet}
				<ListView
					flush
					paginated={false}
					label={t('hosts.pending.title')}
					items={pending}
					itemKey={(token) => token.id}
					icon={() => ({ name: ENTITY_ICONS.enrollmentToken })}
					title={(token) => `${token.display_prefix}…`}
					titleMono
					meta={tokenMeta}
					actions={(token) =>
						canWrite
							? [
									{
										id: 'cancel',
										label: t('hosts.pending.cancelFor', { prefix: token.display_prefix }),
										hint: t('hosts.pending.cancel'),
										icon: 'x',
										danger: true,
										onSelect: () => openCancel(token)
									}
								]
							: []}
				/>
			</Panel>
		</div>
	{/if}

	{#if totalCapped && !pageLoading}
		<div class="capped">
			<Alert tone="info" title={t('filter.capped', { count: formatCount(total) })} />
		</div>
	{/if}

	<ListView
		label={t('hosts.title')}
		items={pageRows}
		itemKey={(agent) => agent.id}
		icon={(agent) => iconOf(hostState(agent))}
		pill={(agent) => pillOf(hostState(agent))}
		title={(agent) => agent.hostname}
		href={(agent) => resolve('/(app)/hosts/[id]', { id: agent.id })}
		{meta}
		actions={rowActions}
		moreLabel={(agent) => t('hosts.moreActions', { name: agent.hostname })}
		mode="server"
		page={tablePage}
		{pageSize}
		pageSizes={PAGE_SIZES}
		{total}
		{totalCapped}
		onPageChange={(next) => filters.update({ page: String(next) })}
		onPageSizeChange={(next) => filters.update({ size: String(next) })}
		loading={loading || pageLoading}
		error={error ?? pageError}
		filtered={!firstRun}
		onReset={resetFilters}
		emptyIcon="host"
		emptyTitle={t('hosts.emptyTitle')}
		emptyDescription={t('hosts.emptyDescription')}
	>
		{#snippet emptyActions()}
			{#if canWrite}
				<Button variant="primary" onclick={openAdd}>{t('hosts.addFirst')}</Button>
			{/if}
		{/snippet}
		{#snippet toolbar()}
			<FilterBar
				label={t('hosts.filters')}
				value={filters.value}
				search={{ key: 'q', label: t('hosts.search'), placeholder: t('hosts.searchPlaceholder') }}
				{states}
				{facets}
				{sort}
				onchange={filters.update}
				onreset={resetFilters}
			/>
		{/snippet}
	</ListView>

	<Dialog
		bind:open={addOpen}
		size={createdToken && !registeredHost ? 'lg' : 'md'}
		title={registeredHost
			? t('hosts.registeredTitle')
			: createdToken
				? t('hosts.tokenTitle')
				: t('hosts.addTitle')}
		description={createdToken ? undefined : t('hosts.addDescription')}
		onsubmit={createdToken ? closeAdd : createToken}
	>
		{#if registeredHost}
			<Alert
				tone="success"
				icon="check"
				title={t('hosts.registered', { name: registeredHost.hostname })}
			/>
		{:else if createdToken}
			<Alert tone="attention" icon="key" title={t('tokens.valueOnceBody')}>
				{t('hosts.tokenNote')}
			</Alert>
			<CopyField value={createdToken.token} label={t('hosts.tokenValue')} secret />
			<CopyField value={installCommand} label={t('hosts.installCommand')} />
		{:else}
			{#if createError}
				<Alert tone="danger" title={createError} />
			{/if}
			<TagInput
				id="host-labels"
				label={t('hosts.labels')}
				error={fieldError(createProblem, 'labels')}
				bind:value={labels}
				placeholder={t('hosts.labelsPlaceholder')}
				mono
				maxLength={MAX_LABEL}
				check={checkLabel}
			/>
		{/if}
		{#snippet footer()}
			{#if createdToken}
				<Button type="submit" variant="primary">{t('common.done')}</Button>
			{:else}
				<Button onclick={closeAdd}>{t('common.cancel')}</Button>
				<Button type="submit" variant="primary" loading={creating}>
					{t('hosts.createToken')}
				</Button>
			{/if}
		{/snippet}
	</Dialog>

	<ConfirmDialog
		bind:open={cancelOpen}
		title={t('hosts.pending.cancelTitle')}
		message={t('hosts.pending.cancelBody', { prefix: cancelTarget?.display_prefix ?? '' })}
		confirmLabel={t('hosts.pending.cancel')}
		cancelLabel={t('hosts.pending.keep')}
		tone="danger"
		loading={cancelling}
		onConfirm={confirmCancel}
	/>

	<HostActions bind:this={hostActions} onchanged={load} />
</Page>

<style>
	.pending__title {
		display: flex;
		align-items: center;
		gap: var(--rh-space-2);
		margin: 0;
		font-size: var(--rh-font-size-sm);
		font-weight: var(--rh-font-weight-semibold);
	}

	.pending__count {
		min-width: 20px;
		padding: 0 6px;
		border-radius: var(--rh-radius-pill);
		background-color: var(--rh-color-neutral-bg);
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-xs);
		line-height: 18px;
		text-align: center;
	}

	.pending,
	.capped {
		margin-bottom: var(--rh-space-4);
	}
</style>
