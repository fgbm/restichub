<script lang="ts">
	import { onMount } from 'svelte';
	import { ApiError, fieldError, service } from '#lib/api/index.ts';
	import type {
		Agent,
		EnrollmentToken,
		EnrollmentTokenCreated,
		ApiProblem
	} from '#lib/api/index.ts';
	import {
		Alert,
		Badge,
		Button,
		ConfirmDialog,
		CopyField,
		Dialog,
		DropdownMenu,
		EmptyState,
		Icon,
		PageHeader,
		Panel,
		StatusPill,
		Table,
		TextField
	} from '#lib/components/ui/index.ts';
	import type { TableColumn } from '#lib/components/ui/index.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import type { MessageKey } from '#lib/i18n/index.ts';
	import { formatTime, relativeTime } from '#lib/format.ts';
	import { enableCommand } from '#lib/scripts.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';

	// The host name the control plane accepts (validate_display_name).
	const HOSTNAME_MAX_LENGTH = 128;

	let agents = $state<Agent[]>([]);
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

	let renameTarget = $state<Agent | null>(null);
	let renameOpen = $state(false);
	let renaming = $state(false);
	let renameName = $state('');
	let renameError = $state<string | null>(null);
	let renameProblem = $state<ApiProblem | null>(null);

	let revokeTarget = $state<Agent | null>(null);
	let revokeOpen = $state(false);
	let revoking = $state(false);

	let reregisterTarget = $state<Agent | null>(null);
	let reregisterOpen = $state(false);
	let reregistering = $state(false);
	let reregisterToken = $state<EnrollmentTokenCreated | null>(null);
	let reregisterTokenOpen = $state(false);

	let replaceTarget = $state<Agent | null>(null);
	let replaceOpen = $state(false);
	let replacing = $state(false);

	const canWrite = $derived(getSession()?.role === 'admin');
	// The registration command carries this installation URL (ADR-0032).
	let origin = $state('');
	onMount(() => {
		origin = window.location.origin;
	});
	const installCommand = $derived(
		t('hosts.installPlaceholder', { url: origin, token: createdToken?.token ?? '<token>' })
	);
	const reregisterCommand = $derived(
		t('hosts.reregisterCommand', { url: origin, token: reregisterToken?.token ?? '<token>' })
	);

	// The dialog shows the host when the agent used the token that the dialog created.
	const registeredHost = $derived.by(() => {
		if (!createdToken) return null;
		const used = enrollmentTokens.find((token) => token.id === createdToken?.id);
		if (!used?.used_at || !used.used_by_agent_id) return null;
		return agents.find((agent) => agent.id === used.used_by_agent_id) ?? null;
	});

	function replacesName(token: EnrollmentToken): string | null {
		if (!token.replaces_agent_id) return null;
		return agents.find((agent) => agent.id === token.replaces_agent_id)?.hostname ?? null;
	}

	// A token stays in the pending list only while it can still enroll a host (FR-ID-01).
	const pending = $derived(
		enrollmentTokens.filter(
			(token) =>
				!token.used_at && !token.revoked_at && new Date(token.expires_at).getTime() > Date.now()
		)
	);

	const renameLength = $derived(Array.from(renameName.trim()).length);
	const renameValid = $derived(renameLength >= 1 && renameLength <= HOSTNAME_MAX_LENGTH);
	const renameFieldError = $derived(
		renameLength > HOSTNAME_MAX_LENGTH
			? t('hosts.renameTooLong')
			: fieldError(renameProblem, 'hostname')
	);

	const columns = $derived<TableColumn<Agent>[]>([
		{
			key: 'hostname',
			header: t('hosts.host'),
			value: (row) => `${row.hostname} ${row.os} ${row.arch}`,
			render: hostCell
		},
		{
			key: 'status',
			header: t('hosts.status'),
			value: (row) => statusLabel(row),
			render: statusCell
		},
		{
			key: 'server_scripts',
			header: t('hosts.serverScripts'),
			value: (row) => scriptsLabel(row),
			render: scriptsCell
		},
		{
			key: 'agent_version',
			header: t('hosts.agent'),
			value: (row) => row.agent_version,
			render: versionCell
		},
		{
			key: 'last_seen_at',
			header: t('hosts.lastSeen'),
			value: (row) => row.last_seen_at ?? '',
			render: lastSeenCell
		},
		{
			key: 'certificate_expires_at',
			header: t('hosts.certificate'),
			value: (row) => row.certificate_expires_at,
			render: certificateCell
		},
		{ key: 'created_at', header: t('hosts.created'), render: createdCell }
	]);

	let scriptsTarget = $state<Agent | null>(null);
	let scriptsOpen = $state(false);

	function scriptsLabel(agent: Agent): string {
		return t(`hosts.serverScripts.${agent.server_scripts}` as MessageKey);
	}

	function openScripts(agent: Agent): void {
		scriptsTarget = agent;
		scriptsOpen = true;
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

	function statusLabel(agent: Agent): string {
		if (agent.status === 'revoked') return t('hosts.status.revoked');
		return t(isWaiting(agent) ? 'hosts.status.waiting' : 'hosts.status.active');
	}

	// The token row shows the creator's name when the caller created it (getSession is the caller).
	function createdByLabel(token: EnrollmentToken): string {
		const user = getSession()?.user;
		return user && user.id === token.created_by ? user.display_name : token.created_by;
	}

	const empty = $derived(!loading && !error && agents.length === 0);

	function messageOf(problem: unknown): string {
		if (problem instanceof ApiError) return tErrorCode(problem.code) ?? problem.title;
		return t('table.error');
	}

	async function load(): Promise<void> {
		loading = true;
		error = null;
		try {
			const [agentList, tokenList] = await Promise.all([
				service.listAgents(),
				service.listEnrollmentTokens()
			]);
			agents = agentList;
			enrollmentTokens = tokenList;
		} catch (problem) {
			error = messageOf(problem);
		} finally {
			loading = false;
		}
	}

	onMount(load);

	// A used token leaves the pending list and a new host appears without a manual reload.
	const REFRESH_MS = 10_000;

	async function refresh(): Promise<void> {
		try {
			const [agentList, tokenList] = await Promise.all([
				service.listAgents(),
				service.listEnrollmentTokens()
			]);
			agents = agentList;
			enrollmentTokens = tokenList;
		} catch {
			// The next tick tries again; the visible list stays.
		}
	}

	onMount(() => {
		const timer = setInterval(() => {
			if (!document.hidden && !loading) void refresh();
		}, REFRESH_MS);
		return () => clearInterval(timer);
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

	function openReregister(agent: Agent): void {
		reregisterTarget = agent;
		reregisterOpen = true;
	}

	async function confirmReregister(): Promise<void> {
		if (!reregisterTarget) return;
		reregistering = true;
		try {
			reregisterToken = await service.reenrollAgent(reregisterTarget.id);
			reregisterOpen = false;
			reregisterTokenOpen = true;
			await load();
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		} finally {
			reregistering = false;
		}
	}

	$effect(() => {
		if (!reregisterTokenOpen) reregisterToken = null;
	});

	function openReplace(agent: Agent): void {
		replaceTarget = agent;
		replaceOpen = true;
	}

	async function confirmReplace(): Promise<void> {
		const old = replaceTarget?.possible_duplicate_of;
		if (!replaceTarget || !old) return;
		replacing = true;
		try {
			const moved = await service.replaceAgent(replaceTarget.id, old.id);
			pushToast('success', t('hosts.replaced', { plans: moved.plans_moved }));
			replaceOpen = false;
			replaceTarget = null;
			await load();
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		} finally {
			replacing = false;
		}
	}

	function openRename(agent: Agent): void {
		renameTarget = agent;
		renameName = agent.hostname;
		renameError = null;
		renameProblem = null;
		renameOpen = true;
	}

	async function renameHost(): Promise<void> {
		if (!renameTarget || !renameValid) return;
		renaming = true;
		renameError = null;
		renameProblem = null;
		try {
			await service.renameAgent(renameTarget.id, renameName.trim());
			pushToast('success', t('hosts.renamed'));
			renameOpen = false;
			await load();
		} catch (problem) {
			renameProblem = problem instanceof ApiError ? problem.problem : null;
			renameError =
				renameLength > HOSTNAME_MAX_LENGTH || fieldError(renameProblem, 'hostname')
					? null
					: messageOf(problem);
		} finally {
			renaming = false;
		}
	}

	async function confirmRevoke(): Promise<void> {
		if (!revokeTarget) return;
		revoking = true;
		try {
			await service.revokeAgent(revokeTarget.id);
			pushToast('success', t('hosts.revoked'));
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

{#snippet timeCell(value: string | null | undefined)}
	{#if value}
		<time datetime={value} title={formatTime(value)}>{relativeTime(value)}</time>
	{:else}
		{t('common.never')}
	{/if}
{/snippet}

{#snippet lastSeenCell(agent: Agent)}
	<span class="nowrap">{@render timeCell(agent.last_seen_at)}</span>
{/snippet}

{#snippet createdCell(agent: Agent)}
	<span class="nowrap">{@render timeCell(agent.created_at)}</span>
{/snippet}

{#snippet hostCell(agent: Agent)}
	<span class="host">
		<span class="host__name">{agent.hostname}</span>
		<span class="host__meta">{agent.os} · {agent.arch}</span>
	</span>
{/snippet}

{#snippet statusCell(agent: Agent)}
	<span class="status">
		<span title={isWaiting(agent) ? t('hosts.waitingTitle') : undefined}>
			<StatusPill
				tone={agent.status === 'revoked' ? 'neutral' : isWaiting(agent) ? 'info' : 'success'}
			>
				{statusLabel(agent)}
			</StatusPill>
		</span>
		{#if agent.possible_duplicate_of && agent.status === 'active'}
			<span title={t('hosts.duplicateTitle', { name: agent.possible_duplicate_of.hostname })}>
				<Badge tone="warning">
					{t('hosts.duplicate')}
				</Badge>
			</span>
		{/if}
	</span>
{/snippet}

{#snippet scriptsCell(agent: Agent)}
	<button
		type="button"
		class="host__scripts"
		aria-label={`${t('hosts.serverScripts')}: ${agent.hostname}`}
		onclick={() => openScripts(agent)}
	>
		<StatusPill tone={agent.server_scripts === 'enabled' ? 'info' : 'neutral'}>
			{scriptsLabel(agent)}
		</StatusPill>
	</button>
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

<svelte:head>
	<title>{t('hosts.title')} · {t('app.name')}</title>
</svelte:head>

<PageHeader
	title={t('hosts.title')}
	description={t('hosts.description')}
	count={loading ? undefined : agents.length}
>
	{#if canWrite}
		<Button variant="primary" icon="plus" onclick={openAdd}>{t('hosts.add')}</Button>
	{/if}
</PageHeader>

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
								<Button size="sm" onclick={() => openCancel(token)}>
									{t('hosts.pending.cancel')}
								</Button>
							</span>
						{/if}
					</li>
				{/each}
			</ul>
		</Panel>
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
	<Table
		{columns}
		rows={agents}
		{loading}
		{error}
		rowKey={(agent) => agent.id}
		filterPlaceholder={t('hosts.searchPlaceholder')}
	>
		{#snippet rowActions(agent)}
			{#if canWrite && agent.status === 'active'}
				<DropdownMenu
					icon="more"
					align="end"
					label={t('hosts.moreActions', { name: agent.hostname })}
					items={[
						{
							label: t('hosts.rename'),
							icon: 'tag',
							onSelect: () => openRename(agent)
						},
						{
							label: t('hosts.reregister'),
							icon: 'key',
							onSelect: () => openReregister(agent)
						},
						...(agent.possible_duplicate_of?.is_older
							? [
									{
										label: t('hosts.replaceOld'),
										icon: 'host' as const,
										onSelect: () => openReplace(agent)
									}
								]
							: []),
						{
							label: t('hosts.revokeAgent'),
							icon: 'octagon',
							tone: 'danger',
							separatorBefore: true,
							onSelect: () => {
								revokeTarget = agent;
								revokeOpen = true;
							}
						}
					]}
				/>
			{/if}
		{/snippet}
	</Table>
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

<Dialog
	bind:open={scriptsOpen}
	size="md"
	title={t('hosts.serverScripts')}
	description={scriptsTarget ? scriptsTarget.hostname : undefined}
>
	{#if scriptsTarget}
		<dl class="scripts">
			<dt>{t('hosts.serverScripts')}</dt>
			<dd>{scriptsLabel(scriptsTarget)}</dd>
			<dt>{t('hosts.serverScripts.interpreters')}</dt>
			<dd>
				{scriptsTarget.script_interpreters.length > 0
					? scriptsTarget.script_interpreters.join(', ')
					: t('common.none')}
			</dd>
		</dl>
		{#if scriptsTarget.server_scripts === 'enabled'}
			<Alert
				variant="inline"
				tone="attention"
				icon="shield"
				title={t('hosts.serverScripts.risk')}
			/>
		{:else}
			<p class="scripts__hint">{t('hosts.serverScripts.enableHint')}</p>
			<CopyField value={enableCommand(scriptsTarget.os)} />
		{/if}
	{/if}
	{#snippet footer()}
		<Button onclick={() => (scriptsOpen = false)}>{t('common.close')}</Button>
	{/snippet}
</Dialog>

<ConfirmDialog
	bind:open={reregisterOpen}
	title={t('hosts.reregisterTitle')}
	message={t('hosts.reregisterBody', { name: reregisterTarget?.hostname ?? '' })}
	confirmLabel={t('hosts.reregisterCreate')}
	cancelLabel={t('common.cancel')}
	loading={reregistering}
	onConfirm={confirmReregister}
/>

<Dialog bind:open={reregisterTokenOpen} size="lg" title={t('hosts.reregisterTokenTitle')}>
	{#if reregisterToken}
		<Alert tone="attention" icon="key" title={t('tokens.valueOnceBody')}>
			{t('hosts.reregisterNote', { name: reregisterTarget?.hostname ?? '' })}
		</Alert>
		<CopyField value={reregisterToken.token} label={t('hosts.tokenValue')} secret />
		<CopyField value={reregisterCommand} label={t('hosts.installCommand')} />
	{/if}
	{#snippet footer()}
		<Button variant="primary" onclick={() => (reregisterTokenOpen = false)}>
			{t('common.done')}
		</Button>
	{/snippet}
</Dialog>

<ConfirmDialog
	bind:open={replaceOpen}
	title={t('hosts.replaceTitle')}
	message={t('hosts.replaceBody', {
		old: replaceTarget?.possible_duplicate_of?.hostname ?? '',
		oldSeen: replaceTarget?.possible_duplicate_of?.last_seen_at
			? relativeTime(replaceTarget.possible_duplicate_of.last_seen_at)
			: t('common.never'),
		newSeen: replaceTarget?.last_seen_at
			? relativeTime(replaceTarget.last_seen_at)
			: t('common.never')
	})}
	confirmLabel={t('hosts.replace')}
	cancelLabel={t('common.cancel')}
	tone="danger"
	loading={replacing}
	onConfirm={confirmReplace}
/>

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

<Dialog bind:open={renameOpen} size="sm" title={t('hosts.renameTitle')} onsubmit={renameHost}>
	{#if renameError}
		<Alert tone="danger" title={renameError} />
	{/if}
	<TextField
		id="host-rename"
		label={t('hosts.hostname')}
		help={t('hosts.renameHint')}
		error={renameFieldError}
		bind:value={renameName}
		maxlength={HOSTNAME_MAX_LENGTH}
		required
	/>
	{#snippet footer()}
		<Button onclick={() => (renameOpen = false)}>{t('common.cancel')}</Button>
		<Button type="submit" variant="primary" loading={renaming} disabled={!renameValid}>
			{t('hosts.rename')}
		</Button>
	{/snippet}
</Dialog>

<Dialog bind:open={revokeOpen} size="sm" icon="octagon" title={t('hosts.revokeTitle')}>
	<p>
		{t('hosts.revokeBody', { name: revokeTarget?.hostname ?? '' })}
	</p>
	{#snippet footer()}
		<Button onclick={() => (revokeOpen = false)}>{t('common.cancel')}</Button>
		<Button variant="danger" loading={revoking} onclick={confirmRevoke}>
			{t('hosts.revoke')}
		</Button>
	{/snippet}
</Dialog>

<style>
	.host__scripts {
		padding: 0;
		border: 0;
		background: none;
		cursor: pointer;
	}

	.scripts {
		display: grid;
		grid-template-columns: max-content 1fr;
		gap: var(--rh-space-2) var(--rh-space-4);
		margin: 0 0 var(--rh-space-3);
	}

	.scripts dt {
		color: var(--rh-color-text-muted);
	}

	.scripts dd {
		margin: 0;
	}

	.scripts__hint {
		margin: 0 0 var(--rh-space-2);
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.status {
		display: inline-flex;
		flex-direction: column;
		align-items: flex-start;
		gap: var(--rh-space-1);
	}

	.nowrap {
		white-space: nowrap;
	}

	.host {
		display: flex;
		flex-direction: column;
	}

	.host__name {
		font-weight: var(--rh-font-weight-semibold);
		white-space: nowrap;
	}

	.host__meta {
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
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

	.pending {
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
