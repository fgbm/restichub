<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { ApiError, fieldError, service } from '#lib/api/index.ts';
	import type { ApiProblem } from '#lib/api/index.ts';
	import type { Agent, OverwriteMode, Repository, SnapshotItem } from '#lib/api/index.ts';
	import { Alert, Button, Checkbox, Dialog, Select, TextField } from '#lib/components/ui/index.ts';
	import type { SelectOption } from '#lib/components/ui/index.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import type { MessageKey } from '#lib/i18n/index.ts';
	import { OVERWRITE_MODES, isAbsolutePath, isSystemPath } from '#lib/restore.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';

	let {
		open = $bindable(false),
		repository,
		snapshot,
		paths,
		agents
	}: {
		open?: boolean;
		repository: Repository;
		snapshot: SnapshotItem;
		/** The selected paths. An empty list restores the whole snapshot. */
		paths: string[];
		agents: Agent[];
	} = $props();

	let agentId = $state('');
	let target = $state('');
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
	const system = $derived(trimmed !== '' && !relative && isSystemPath(trimmed));
	const windows = $derived(candidates.find((agent) => agent.id === agentId)?.os === 'windows');

	$effect(() => {
		if (!open) return;
		// Each opening starts from the defaults, so a finished restore leaves nothing behind.
		agentId =
			candidates.find((agent) => agent.id === repository.maintenance_agent_id)?.id ??
			candidates[0]?.id ??
			'';
		target = '';
		overwrite = 'never';
		dryRun = false;
		verify = false;
		problem = null;
		failure = null;
	});

	async function submit(): Promise<void> {
		if (!agentId || trimmed === '' || relative) return;
		submitting = true;
		problem = null;
		failure = null;
		try {
			const job = await service.restoreSnapshot(repository.id, {
				agent_id: agentId,
				snapshot_id: snapshot.id,
				includes: paths,
				excludes: [],
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
	<p class="scope">
		{paths.length === 0
			? t('restore.dialog.scopeAll')
			: t('restore.dialog.scopeSome', { count: paths.length })}
		· {snapshot.hostname} · {repository.name}
	</p>
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
		<TextField
			id="restore-target"
			label={t('restore.dialog.target')}
			help={t('restore.dialog.targetHelp')}
			placeholder={windows ? 'D:\\restore' : t('restore.dialog.targetPlaceholder')}
			error={relative ? t('restore.dialog.targetRelative') : fieldError(problem, 'target')}
			mono
			autocomplete="off"
			bind:value={target}
			required
		/>
		{#if system}
			<Alert tone="attention" icon="alert" title={t('restore.dialog.systemPath')}>
				{t('restore.dialog.systemPathBody')}
			</Alert>
		{/if}
		<Select
			id="restore-overwrite"
			label={t('restore.dialog.overwrite')}
			error={fieldError(problem, 'overwrite')}
			options={overwriteOptions}
			bind:value={overwrite}
		/>
		<div class="options">
			<Checkbox
				id="restore-dry-run"
				label={t('restore.dialog.dryRun')}
				help={t('restore.dialog.dryRunHelp')}
				bind:checked={dryRun}
			/>
			<Checkbox
				id="restore-verify"
				label={t('restore.dialog.verify')}
				help={t('restore.dialog.verifyHelp')}
				bind:checked={verify}
			/>
		</div>
	</div>
	{#snippet footer()}
		<Button onclick={() => (open = false)}>{t('common.cancel')}</Button>
		<Button
			type="submit"
			variant="primary"
			loading={submitting}
			disabled={!agentId || trimmed === '' || relative}
		>
			{t('restore.dialog.submit')}
		</Button>
	{/snippet}
</Dialog>

<style>
	.scope {
		margin: 0 0 var(--rh-space-4);
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.form {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-4);
	}

	.options {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-3);
	}
</style>
