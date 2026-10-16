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
		Badge,
		Button,
		ConfirmDialog,
		CopyField,
		DataTable,
		Dialog,
		EmptyState,
		FilterBar,
		Icon,
		Panel,
		StatusPill,
		TextField
	} from '#lib/components/ui/index.ts';
	import type { FilterFacet, RowAction, TableColumn, TableSort } from '#lib/components/ui/index.ts';
	import { queryFilters } from '#lib/stores/query-filters.svelte.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import type { MessageKey } from '#lib/i18n/index.ts';
	import { formatCount, formatTime, relativeTime } from '#lib/format.ts';
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
	let labels = $state('');
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

	const columns = $derived<TableColumn<Agent>[]>([
		{ key: 'hostname', header: t('hosts.host'), truncate: 'sm' },
		{
			key: 'status',
			header: t('hosts.status'),
			sortable: false,
			value: (row) => statusLabel(row),
			render: statusCell
		},
		{
			key: 'os',
			header: t('hosts.system'),
			hideNarrow: true,
			nowrap: true,
			value: (row) => `${row.os} · ${row.arch}`
		},
		{
			key: 'server_scripts',
			hideNarrow: true,
			sortable: false,
			header: t('hosts.serverScripts'),
			value: (row) => scriptsLabel(row),
			render: scriptsCell
		},
		{
			key: 'agent_version',
			hideNarrow: true,
			header: t('hosts.agent'),
			value: (row) => row.agent_version,
			render: versionCell
		},
		{
			key: 'last_seen_at',
			hideNarrow: true,
			header: t('hosts.lastSeen'),
			date: 'relative',
			value: (row) => row.last_seen_at ?? '',
			cell: () => t('common.never')
		},
		{
			key: 'certificate_expires_at',
			hideNarrow: true,
			sortable: false,
			header: t('hosts.certificate'),
			value: (row) => row.certificate_expires_at,
			render: certificateCell
		}
	]);

	type HostState = 'online' | 'offline' | 'waiting' | 'revoked';

	function stateOf(agent: Agent): HostState {
		if (agent.status === 'revoked') return 'revoked';
		if (isWaiting(agent)) return 'waiting';
		return agent.online ? 'online' : 'offline';
	}

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
		os: 'os',
		agent_version: 'agent_version',
		last_seen_at: 'last_seen_at'
	};
	const tablePage = $derived(pageNumber(filters.value.page));
	const pageSize = $derived(pageSizeOf(filters.value.size));
	const tableSort = $derived<TableSort | undefined>(
		filters.value.sort
			? { key: filters.value.sort, direction: filters.value.order === 'asc' ? 'asc' : 'desc' }
			: undefined
	);

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
			sort: SORT_KEYS[value.sort],
			order: value.sort ? (value.order === 'asc' ? 'asc' : 'desc') : undefined,
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
			key: 'state',
			label: t('hosts.status'),
			options: (['online', 'offline', 'waiting', 'revoked'] as const).map((value) => ({
				value,
				label: stateLabel(value),
				count: facetValues?.states[value]
			}))
		},
		{
			key: 'os',
			label: t('hosts.os'),
			options: (facetValues?.os ?? []).map(({ value, count }) => ({ value, count }))
		},
		{
			key: 'agent_version',
			label: t('hosts.agent'),
			options: (facetValues?.agent_versions ?? []).map(({ value, count }) => ({ value, count }))
		},
		{
			key: 'label',
			label: t('hosts.labels'),
			options: (facetValues?.labels ?? []).map(({ value, count }) => ({ value, count }))
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
				icon: 'octagon',
				danger: true,
				onSelect: () => hostActions.openRevoke(agent)
			}
		];
	}

	function scriptsLabel(agent: Agent): string {
		return t(`hosts.serverScripts.${agent.server_scripts}` as MessageKey);
	}

	// An agent rotates at two thirds of the lifetime, so a certificate that is close to its end
	// means that the agent could not rotate (FR-ID-03).
	const CERT_WARNING_MS = 3 * 24 * 60 * 60 * 1000;

	function certificateExpiry(agent: Agent): { expired: boolean; warn: boolean } {
		const left = new Date(agent.certificate_expires_at).getTime() - Date.now();
		return { expired: left <= 0, warn: agent.status === 'active' && left < CERT_WARNING_MS };
	}

	function isWaiting(agent: Agent): boolean {
		return agent.status === 'active' && !agent.first_connected_at;
	}

	function stateLabel(state: HostState): string {
		if (state === 'revoked') return t('hosts.status.revoked');
		if (state === 'waiting') return t('hosts.status.waiting');
		return t(state === 'online' ? 'hp.state.online' : 'hp.state.offline');
	}

	function statusLabel(agent: Agent): string {
		return stateLabel(stateOf(agent));
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
	const empty = $derived(!loading && !error && !narrowed && hostCount === 0);

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

	function openAdd(): void {
		labels = '';
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
			const parsed = labels
				.split(',')
				.map((label) => label.trim())
				.filter((label) => label.length > 0);
			createdToken = await service.createEnrollmentToken({
				labels: parsed.length > 0 ? parsed : undefined
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

{#snippet statusCell(agent: Agent)}
	<span class="status">
		<span title={isWaiting(agent) ? t('hosts.waitingTitle') : undefined}>
			<StatusPill
				tone={agent.status === 'revoked' || (!agent.online && !isWaiting(agent))
					? 'neutral'
					: isWaiting(agent)
						? 'info'
						: 'success'}
			>
				{statusLabel(agent)}
			</StatusPill>
		</span>
		<!-- AC-ID-09: both records of a duplicate show the warning in the list. -->
		{#if agent.possible_duplicate_of && agent.status === 'active'}
			<span title={t('hosts.duplicateTitle', { name: agent.possible_duplicate_of.hostname })}>
				<Badge tone="warning">{t('hosts.duplicate')}</Badge>
			</span>
		{/if}
	</span>
{/snippet}

{#snippet scriptsCell(agent: Agent)}
	<StatusPill tone={agent.server_scripts === 'enabled' ? 'info' : 'neutral'}>
		{scriptsLabel(agent)}
	</StatusPill>
{/snippet}

{#snippet versionCell(agent: Agent)}
	<span class="version">
		<span
			class="host__version"
			title={t('hosts.versionTitle', {
				protocol: agent.protocol_version ?? '-',
				version: agent.control_plane_version
			})}>{agent.agent_version}</span
		>
		{#if agent.version_status === 'update_available' && agent.status !== 'revoked'}
			<Badge tone="warning">{t('hosts.versionUpdate')}</Badge>
		{:else if agent.version_status === 'unsupported'}
			<Badge tone="danger">{t('hosts.versionUnsupported')}</Badge>
		{/if}
	</span>
{/snippet}

{#snippet certificateCell(agent: Agent)}
	{@const expiry = certificateExpiry(agent)}
	<span class="cert">
		<time
			class="cert__expiry"
			class:cert__expiry--warn={expiry.warn}
			datetime={agent.certificate_expires_at}
			title={formatTime(agent.certificate_expires_at)}
		>
			{t(expiry.expired ? 'hosts.certExpired' : 'hosts.certExpires', {
				time: relativeTime(agent.certificate_expires_at)
			})}
		</time>
		<span class="cert__rotation">
			{#if agent.last_certificate_rotation_at}
				{t('hosts.certRotated', { time: relativeTime(agent.last_certificate_rotation_at) })}
			{:else}
				{t('hosts.certNotRotated')}
			{/if}
		</span>
		{#if agent.superseded_certificate_used_at}
			<Badge tone="danger">
				<span title={formatTime(agent.superseded_certificate_used_at)}
					>{t('hosts.certSuperseded')}</span
				>
			</Badge>
		{/if}
	</span>
{/snippet}

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
			<Panel title={t('hosts.pending.title')} subtitle={t('hosts.pending.subtitle')} flush>
				<ul class="pending__list">
					{#each pending as token (token.id)}
						<li class="pending__row">
							<span class="pending__icon"><Icon name="key" /></span>
							<span class="pending__body">
								<span class="pending__head">
									<span class="pending__prefix">{token.display_prefix}</span>
									{#each token.labels as label (label)}
										<Badge tone="neutral">{label}</Badge>
									{/each}
									{#if token.replaces_agent_id}
										<Badge tone="info">
											{t('hosts.pending.reregisters', {
												name: replacesName(token) ?? token.replaces_agent_id
											})}
										</Badge>
									{/if}
								</span>
								<span class="pending__meta">
									{t('hosts.pending.meta', {
										name: createdByLabel(token),
										created: relativeTime(token.created_at),
										expires: relativeTime(token.expires_at)
									})}
								</span>
							</span>
							{#if canWrite}
								<span class="pending__action">
									<Button
										size="sm"
										icon="x"
										title={t('hosts.pending.cancel')}
										aria-label={t('hosts.pending.cancelFor', { prefix: token.display_prefix })}
										onclick={() => openCancel(token)}
									/>
								</span>
							{/if}
						</li>
					{/each}
				</ul>
			</Panel>
		</div>
	{/if}

	{#if totalCapped && !pageLoading}
		<div class="capped">
			<Alert tone="info" title={t('filter.capped', { count: formatCount(total) })} />
		</div>
	{/if}

	{#if empty}
		<EmptyState
			icon="host"
			title={t('hosts.emptyTitle')}
			description={t('hosts.emptyDescription')}
			bordered
		>
			{#if canWrite}
				<Button variant="primary" onclick={openAdd}>{t('hosts.addFirst')}</Button>
			{/if}
		</EmptyState>
	{:else}
		<DataTable
			{columns}
			rows={pageRows}
			mode="server"
			page={tablePage}
			{pageSize}
			pageSizes={PAGE_SIZES}
			{total}
			{totalCapped}
			sort={tableSort}
			onPageChange={(next) => filters.update({ page: String(next) })}
			onPageSizeChange={(next) => filters.update({ size: String(next) })}
			onSortChange={(next) => filters.update({ sort: next.key, order: next.direction })}
			loading={loading || pageLoading}
			error={error ?? pageError}
			rowKey={(agent) => agent.id}
			rowHref={(agent) => resolve('/(app)/hosts/[id]', { id: agent.id })}
			actions={rowActions}
			moreLabel={(agent) => t('hosts.moreActions', { name: agent.hostname })}
			emptyTitle={t('filter.noMatches')}
			emptyDescription={t('filter.noMatchesDescription')}
		>
			{#snippet toolbar()}
				<FilterBar
					label={t('hosts.filters')}
					value={filters.value}
					search={{ key: 'q', label: t('hosts.search'), placeholder: t('hosts.searchPlaceholder') }}
					{facets}
					onchange={filters.update}
					onreset={filters.reset}
				/>
			{/snippet}
		</DataTable>
	{/if}

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
			<TextField
				id="host-labels"
				label={t('hosts.labels')}
				error={fieldError(createProblem, 'labels')}
				bind:value={labels}
				placeholder={t('hosts.labelsPlaceholder')}
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
	.status {
		display: inline-flex;
		align-items: center;
		gap: var(--rh-space-1);
		white-space: nowrap;
	}

	/* A narrow phone has no room for a long state on one line. */
	@media (max-width: 720px) {
		.status,
		.status :global(.status) {
			flex-wrap: wrap;
			white-space: normal;
		}
	}

	.host__version {
		font-family: var(--rh-font-mono);
		font-size: var(--rh-font-size-sm);
	}

	.version {
		display: inline-flex;
		flex-wrap: wrap;
		align-items: center;
		gap: var(--rh-space-2);
	}

	.cert {
		display: flex;
		flex-direction: column;
		white-space: nowrap;
	}

	.cert__expiry--warn {
		color: var(--rh-color-danger-text);
		font-weight: var(--rh-font-weight-semibold);
	}

	.cert__rotation {
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.pending,
	.capped {
		margin-bottom: var(--rh-space-4);
	}

	.pending__list {
		margin: 0;
		padding: 0;
		list-style: none;
	}

	.pending__row {
		display: flex;
		align-items: center;
		gap: var(--rh-space-3);
		padding: var(--rh-space-3) var(--rh-space-4);
		border-top: var(--rh-border-width) solid var(--rh-color-border-muted);
	}

	.pending__row:first-child {
		border-top: 0;
	}

	.pending__icon {
		display: inline-flex;
		flex: none;
		width: 28px;
		justify-content: center;
		color: var(--rh-color-text-muted);
	}

	.pending__body {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-1);
		min-width: 0;
	}

	.pending__head {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: var(--rh-space-2);
	}

	.pending__prefix {
		font-family: var(--rh-font-mono);
		font-weight: var(--rh-font-weight-semibold);
	}

	.pending__meta {
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.pending__action {
		margin-left: auto;
	}
</style>
