<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { ApiError, fieldError, service } from '#lib/api/index.ts';
	import type { ApiProblem } from '#lib/api/index.ts';
	import type { Agent, OverwriteMode, Repository, SnapshotItem } from '#lib/api/index.ts';
	import {
		Alert,
		Badge,
		Button,
		Checkbox,
		Dialog,
		Field,
		Select,
		TextField
	} from '#lib/components/ui/index.ts';
	import type { SelectOption } from '#lib/components/ui/index.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import type { MessageKey } from '#lib/i18n/index.ts';
	import {
		OVERWRITE_MODES,
		escapeGlob,
		isAbsolutePath,
		isWithinRoots,
		landingPath,
		parsePatterns,
		suggestTarget,
		warnsSystemPath
	} from '#lib/restore.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';

	let {
		open = $bindable(false),
		repository,
		snapshot,
		paths,
		agents,
		preferredAgentId = ''
	}: {
		open?: boolean;
		repository: Repository;
		snapshot: SnapshotItem;
		/** The selected paths. An empty list restores the whole snapshot. */
		paths: string[];
		agents: Agent[];
		/** The host to preselect as the target, when it is an active candidate. */
		preferredAgentId?: string;
	} = $props();

	let agentId = $state('');
	let target = $state('');
	let excludeText = $state('');
	let overwrite = $state<OverwriteMode>('never');
	let dryRun = $state(false);
	let verify = $state(false);
	let submitting = $state(false);
	let problem = $state<ApiProblem | null>(null);
	let failure = $state<string | null>(null);

	// A repository with a local path lives on one host, so only its maintenance host can read it.
	const localOnly = $derived(repository.backend.kind === 'local');
	const candidates = $derived(
		agents.filter(
			(agent) =>
				agent.status === 'active' &&
				(!localOnly || agent.id === (repository.maintenance_agent_id ?? agent.id))
		)
	);
	const agentOptions = $derived<SelectOption[]>(
		candidates.map((agent) => ({ value: agent.id, label: `${agent.hostname} · ${agent.os}` }))
	);
	const overwriteOptions = $derived<SelectOption[]>(
		OVERWRITE_MODES.map((value) => ({
			value,
			label: t(`restore.overwrite.${value}` as MessageKey)
		}))
	);
	const trimmed = $derived(target.trim());
	const relative = $derived(trimmed !== '' && !isAbsolutePath(trimmed));
	const selected = $derived(candidates.find((agent) => agent.id === agentId));
	const windows = $derived(selected?.os === 'windows');
	const roots = $derived(selected?.restore_roots ?? []);
	const suggestion = $derived(suggestTarget(roots[0], snapshot.short_id, windows));
	const outside = $derived(
		trimmed !== '' && !relative && roots.length > 0 && !isWithinRoots(trimmed, roots, windows)
	);
	// A path outside the roots is refused with one message, so the system warning stays for a path
	// that the roots allow.
	const system = $derived(
		trimmed !== '' && !relative && !outside && warnsSystemPath(trimmed, roots, windows)
	);
	const wholeSnapshot = $derived(paths.length === 0);
	const excludes = $derived(wholeSnapshot ? parsePatterns(excludeText) : []);
	const replacesAll = $derived(overwrite === 'always' && !dryRun);
	const landing = $derived(
		trimmed === '' || relative || outside
			? ''
			: landingPath(trimmed, paths[0] ?? snapshot.paths[0] ?? '', windows)
	);
	const mode = $derived(dryRun ? 'modeDryRun' : verify ? 'modeVerify' : 'modeRestore');
	const blocked = $derived(!agentId || trimmed === '' || relative || outside || roots.length === 0);
	let suggested = '';

	// The target follows the suggestion of the selected host until the person types another one.
	$effect(() => {
		if (open && target === suggested) target = suggestion;
		suggested = suggestion;
	});

	$effect(() => {
		if (!open) return;
		// Each opening starts from the defaults, so a finished restore leaves nothing behind.
		// The files of a snapshot usually come back to the host that made it.
		agentId =
			candidates.find((agent) => agent.id === preferredAgentId)?.id ??
			candidates.find((agent) => agent.hostname === snapshot.hostname)?.id ??
			candidates.find((agent) => agent.id === repository.maintenance_agent_id)?.id ??
			candidates[0]?.id ??
			'';
		target = '';
		excludeText = '';
		suggested = '';
		overwrite = 'never';
		dryRun = false;
		verify = false;
		problem = null;
		failure = null;
	});

	async function submit(): Promise<void> {
		if (blocked) return;
		submitting = true;
		problem = null;
		failure = null;
		try {
			const job = await service.restoreSnapshot(repository.id, {
				agent_id: agentId,
				snapshot_id: snapshot.id,
				includes: paths.map(escapeGlob),
				excludes,
				target: trimmed,
				overwrite,
				dry_run: dryRun,
				verify
			});
			pushToast('success', t('restore.dialog.started'));
			open = false;
			await goto(resolve('/(app)/jobs/[id]', { id: job.id }));
		} catch (error) {
			if (error instanceof ApiError) {
				problem = error.problem;
				failure = fieldError(problem, 'agent_id') ? null : (tErrorCode(error.code) ?? error.title);
			} else {
				failure = t('table.error');
			}
		} finally {
			submitting = false;
		}
	}
</script>

<Dialog
	bind:open
	size="lg"
	title={t('restore.dialog.title', { id: snapshot.short_id })}
	description={t('restore.dialog.description')}
	onsubmit={submit}
>
	{#if failure}
		<Alert tone="danger" title={failure} />
	{/if}
	<div class="form">
		<Select
			id="restore-agent"
			label={t('restore.dialog.agent')}
			help={localOnly ? t('restore.dialog.agentLocal') : t('restore.dialog.agentHelp')}
			error={fieldError(problem, 'agent_id')}
			options={agentOptions}
			placeholder={agentOptions.length === 0 ? t('restore.dialog.agentNone') : undefined}
			bind:value={agentId}
			disabled={agentOptions.length === 0}
		/>
		<div class="target">
			<TextField
				id="restore-target"
				label={t('restore.dialog.target')}
				help={t('restore.dialog.targetHelp')}
				placeholder={windows ? 'D:\\restore' : t('restore.dialog.targetPlaceholder')}
				error={relative
					? t('restore.dialog.targetRelative')
					: outside
						? t('restore.dialog.targetOutside')
						: fieldError(problem, 'target')}
				mono
				autocomplete="off"
				bind:value={target}
				required
			/>
			{#if roots.length > 0}
				<div class="roots">
					<span class="roots__label">{t('restore.dialog.rootsLabel')}</span>
					{#each roots as root (root)}
						<Button
							size="sm"
							class="roots__root"
							aria-label={t('restore.dialog.rootUse', { root })}
							onclick={() => (target = suggestTarget(root, snapshot.short_id, windows))}
						>
							<span class="mono">{root}</span>
						</Button>
					{/each}
				</div>
				<p class="hint">{t('restore.dialog.rootsRule')}</p>
			{/if}
		</div>
		{#if agentId && roots.length === 0}
			<Alert tone="attention" icon="alert" title={t('restore.dialog.rootsNone')}>
				{t('restore.dialog.rootsNoneBody')}
				{#snippet actions()}
					<a class="link" href={resolve('/(app)/hosts')}>{t('restore.dialog.hostsLink')}</a>
				{/snippet}
			</Alert>
		{/if}
		{#if system}
			<Alert tone="attention" icon="alert" title={t('restore.dialog.systemPath')}>
				{t('restore.dialog.systemPathBody')}
			</Alert>
		{/if}
		{#if wholeSnapshot}
			<Field
				label={t('restore.dialog.excludes')}
				help={t('restore.dialog.excludesHelp')}
				error={fieldError(problem, 'excludes')}
				for="restore-excludes"
			>
				<textarea
					id="restore-excludes"
					class="rh-textarea rh-input--mono"
					rows="2"
					placeholder={t('restore.dialog.excludesPlaceholder')}
					bind:value={excludeText}></textarea>
			</Field>
		{/if}
		<Select
			id="restore-overwrite"
			label={t('restore.dialog.overwrite')}
			error={fieldError(problem, 'overwrite')}
			options={overwriteOptions}
			bind:value={overwrite}
		/>
		{#if replacesAll}
			<Alert tone="attention" icon="alert" title={t('restore.dialog.overwriteAlwaysTitle')}>
				{t('restore.dialog.overwriteAlwaysBody')}
			</Alert>
		{/if}
		<div class="options">
			<Checkbox
				id="restore-dry-run"
				label={t('restore.dialog.dryRun')}
				help={t('restore.dialog.dryRunHelp')}
				bind:checked={dryRun}
				disabled={verify}
			/>
			<Checkbox
				id="restore-verify"
				label={t('restore.dialog.verify')}
				help={dryRun ? t('restore.dialog.verifyDryRun') : t('restore.dialog.verifyHelp')}
				bind:checked={verify}
				disabled={dryRun}
			/>
		</div>
		<section class="summary" aria-label={t('restore.dialog.summary')}>
			<h3 class="summary__title">{t('restore.dialog.summary')}</h3>
			<dl class="summary__list">
				<dt>{t('restore.dialog.summarySnapshot')}</dt>
				<dd>
					<span class="mono">{snapshot.short_id}</span> · {snapshot.hostname} · {repository.name}
				</dd>
				<dt>{t('restore.dialog.summaryScope')}</dt>
				<dd>
					{#if wholeSnapshot}
						<Badge tone="warning">{t('restore.dialog.wholeSnapshot')}</Badge>
					{:else}
						{t('restore.dialog.scopeSome', { count: paths.length })}
						<span class="summary__paths mono">{paths.join(', ')}</span>
					{/if}
					{#if excludes.length > 0}
						<span class="summary__paths">
							{t('restore.dialog.summaryExcluding', { count: excludes.length })}
						</span>
					{/if}
				</dd>
				<dt>{t('restore.dialog.summaryTarget')}</dt>
				<dd>
					{selected?.hostname ?? '—'}<span class="mono">{trimmed ? `: ${trimmed}` : ''}</span>
				</dd>
				{#if landing}
					<dt>{t('restore.dialog.summaryLands')}</dt>
					<dd class="mono">{landing}</dd>
				{/if}
				<dt>{t('restore.dialog.overwrite')}</dt>
				<dd>{t(`restore.overwrite.${overwrite}` as MessageKey)}</dd>
				<dt>{t('restore.dialog.summaryMode')}</dt>
				<dd>{t(`restore.dialog.${mode}` as MessageKey)}</dd>
			</dl>
		</section>
	</div>
	{#snippet footer()}
		<Button onclick={() => (open = false)}>{t('common.cancel')}</Button>
		<Button type="submit" variant="primary" loading={submitting} disabled={blocked}>
			{t('restore.dialog.submit')}
		</Button>
	{/snippet}
</Dialog>

<style>
	.form {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-4);
	}

	.target {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-2);
	}

	.roots {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: var(--rh-space-2);
	}

	.roots__label {
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.hint {
		margin: 0;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.link {
		color: var(--rh-color-link);
		font-weight: var(--rh-font-weight-semibold);
	}

	.mono {
		font-family: var(--rh-font-mono, monospace);
	}

	.options {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-3);
	}

	.summary {
		padding: var(--rh-space-4);
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-surface-muted, var(--rh-color-canvas-subtle));
	}

	.summary__title {
		margin: 0 0 var(--rh-space-2);
		font-size: var(--rh-font-size-base);
	}

	.summary__list {
		display: grid;
		grid-template-columns: minmax(6rem, 10rem) minmax(0, 1fr);
		gap: var(--rh-space-1) var(--rh-space-3);
		margin: 0;
		font-size: var(--rh-font-size-sm);
	}

	.summary__list dt {
		color: var(--rh-color-text-muted);
	}

	.summary__list dd {
		margin: 0;
		overflow-wrap: anywhere;
	}

	.summary__paths {
		display: block;
		color: var(--rh-color-text-muted);
	}

	@media (max-width: 40rem) {
		.summary__list {
			grid-template-columns: minmax(0, 1fr);
		}
	}
</style>
