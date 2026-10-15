<script lang="ts">
	import { ApiError, fieldError, service } from '#lib/api/index.ts';
	import type { Agent, JobTemplate, ScriptTemplate, Secret } from '#lib/api/index.ts';
	import {
		Alert,
		Badge,
		Button,
		CopyField,
		Dialog,
		DropdownMenu,
		TextField
	} from '#lib/components/ui/index.ts';
	import PasswordField from '#lib/components/ui/PasswordField.svelte';
	import type { MenuItem } from '#lib/components/ui/index.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';
	import type { ApiProblem } from '#lib/api/index.ts';
	import {
		blankScript,
		canMove,
		draftFromTemplate,
		draftOf,
		enableCommand,
		groupOf,
		hostProblem,
		move
	} from '#lib/scripts.ts';
	import type { Group, ScriptDraft } from '#lib/scripts.ts';
	import ScriptCard from './ScriptCard.svelte';

	let {
		scripts = $bindable([]),
		templates = [],
		jobTemplates = [],
		secrets = $bindable([]),
		targets = [],
		readonly = false,
		problem = null,
		framed = true
	}: {
		scripts?: ScriptDraft[];
		templates?: ScriptTemplate[];
		jobTemplates?: JobTemplate[];
		secrets?: Secret[];
		/** The hosts that run the profile: the hosts of its plans. */
		targets?: Agent[];
		/** An Operator and a Viewer read the scripts and change nothing. */
		readonly?: boolean;
		problem?: ApiProblem | null;
		/** Shows the title, the description and the risk text above the lists. */
		framed?: boolean;
	} = $props();

	let templateOpen = $state(false);
	let templateGroup = $state<Group>('before');
	let templateQuery = $state('');
	let jobTemplateOpen = $state(false);

	let secretOpen = $state(false);
	let secretTarget = $state<{ script: number; parameter: number } | null>(null);
	let secretName = $state('');
	let secretValue = $state('');
	let secretSaving = $state(false);
	let secretProblem = $state<ApiProblem | null>(null);
	let secretError = $state<string | null>(null);

	const beforeCount = $derived(scripts.filter((item) => groupOf(item.phase) === 'before').length);
	const afterCount = $derived(scripts.length - beforeCount);

	// The position of each script in its own list, counted from 1.
	const numbers = $derived.by(() => {
		const counters: Record<Group, number> = { before: 0, after: 0 };
		const result: Record<number, number> = {};
		for (const item of scripts) {
			const group = groupOf(item.phase);
			counters[group] += 1;
			result[item.key] = counters[group];
		}
		return result;
	});

	const disabledHosts = $derived(
		scripts.length === 0
			? []
			: targets.filter((agent) => agent.status === 'active' && agent.server_scripts !== 'enabled')
	);
	const missingInterpreterHosts = $derived(
		scripts.length === 0
			? []
			: targets.filter(
					(agent) =>
						agent.status === 'active' &&
						agent.server_scripts === 'enabled' &&
						scripts.some((item) => hostProblem(agent, item.interpreter) === 'interpreter')
				)
	);
	// The command suits the operating systems of the hosts that need it. A profile without a
	// target shows the command of Linux.
	const commandSystems = $derived.by(() => {
		const systems = disabledHosts.map((agent) =>
			agent.os.toLowerCase() === 'windows' ? 'windows' : 'linux'
		);
		const unique = systems.filter((os, index) => systems.indexOf(os) === index);
		return unique.length > 0 ? unique : ['linux'];
	});

	const visibleTemplates = $derived.by(() => {
		const query = templateQuery.trim().toLowerCase();
		return templates.filter(
			(item) =>
				query.length === 0 ||
				item.name.toLowerCase().includes(query) ||
				item.description.toLowerCase().includes(query)
		);
	});

	const addItems = $derived<MenuItem[]>([
		{ label: t('scripts.addBlank'), icon: 'plus', onSelect: () => addBlank('before') },
		{
			label: t('scripts.addTemplate'),
			icon: 'file',
			onSelect: () => openTemplates('before')
		},
		{
			label: t('scripts.useJobTemplate'),
			icon: 'checklist',
			onSelect: () => (jobTemplateOpen = true)
		}
	]);

	function messageOf(error: unknown): string {
		if (error instanceof ApiError) return tErrorCode(error.code) ?? error.title;
		return t('table.error');
	}

	function errorFor(index: number): (field: string) => string | undefined {
		return (field) => fieldError(problem, `scripts[${index}].${field}`);
	}

	function addBlank(group: Group): void {
		scripts = [...scripts, blankScript(group)];
	}

	function openTemplates(group: Group): void {
		templateGroup = group;
		templateQuery = '';
		templateOpen = true;
	}

	function addFromTemplate(template: ScriptTemplate): void {
		const draft = draftFromTemplate(template);
		// The template phase decides the list. A request from the second list asks for that list.
		if (templateGroup === 'after' && groupOf(draft.phase) === 'before') draft.phase = 'after';
		scripts = [...scripts, draft];
		templateOpen = false;
	}

	function addFromJobTemplate(template: JobTemplate): void {
		scripts = [...scripts, ...template.scripts.map((item) => draftOf(item, true))];
		jobTemplateOpen = false;
	}

	function remove(key: number): void {
		scripts = scripts.filter((item) => item.key !== key);
	}

	function openSecret(script: number, parameter: number): void {
		secretTarget = { script, parameter };
		secretName = '';
		secretValue = '';
		secretProblem = null;
		secretError = null;
		secretOpen = true;
	}

	async function createSecret(): Promise<void> {
		secretSaving = true;
		secretProblem = null;
		secretError = null;
		try {
			const created = await service.createSecret({
				name: secretName.trim(),
				purpose: 'hook_secret',
				value: secretValue
			});
			secrets = [...secrets, created];
			if (secretTarget && scripts[secretTarget.script]) {
				scripts[secretTarget.script].env[secretTarget.parameter].value = created.id;
			}
			secretOpen = false;
			secretValue = '';
			pushToast('success', t('scripts.secret.created'));
		} catch (error) {
			if (error instanceof ApiError) secretProblem = error.problem;
			else secretError = messageOf(error);
		} finally {
			secretSaving = false;
		}
	}
</script>

<div class="scripts" data-testid="scripts-editor">
	{#if framed}
		<p class="scripts__text">{t('scripts.description')}</p>
		<Alert variant="inline" tone="attention" icon="shield" title={t('scripts.risk')} />
	{/if}
	{#if readonly}
		<Alert variant="inline" icon="lock" title={t('scripts.readonly')} />
	{/if}

	{#if disabledHosts.length > 0 || missingInterpreterHosts.length > 0}
		<Alert tone="attention" title={t('scripts.warning.title')}>
			{#if disabledHosts.length > 0}
				<p class="warn__text">
					{t('scripts.warning.disabled', {
						hosts: disabledHosts.map((agent) => agent.hostname).join(', ')
					})}
				</p>
			{/if}
			{#if missingInterpreterHosts.length > 0}
				<p class="warn__text">
					{t('scripts.warning.interpreter', {
						hosts: missingInterpreterHosts.map((agent) => agent.hostname).join(', ')
					})}
				</p>
			{/if}
			{#if disabledHosts.length > 0}
				<p class="warn__text">{t('scripts.warning.howTo')}</p>
				{#each commandSystems as system (system)}
					<CopyField value={enableCommand(system)} />
				{/each}
			{/if}
		</Alert>
	{:else if scripts.length > 0 && targets.length === 0}
		<Alert variant="inline" icon="info" title={t('scripts.warning.noHosts')} />
	{/if}

	{#if !readonly}
		<div class="scripts__toolbar">
			<DropdownMenu icon="plus" items={addItems} label={t('scripts.add')}>
				{t('scripts.add')}
			</DropdownMenu>
		</div>
	{/if}

	{#each ['before', 'after'] as const as group (group)}
		<section class="list" data-testid={`scripts-${group}`}>
			<div class="list__head">
				<h4 class="list__title">{t(`scripts.${group}`)}</h4>
				<Badge tone="neutral">{group === 'before' ? beforeCount : afterCount}</Badge>
				{#if !readonly}
					<span class="list__actions">
						<Button size="sm" variant="ghost" icon="plus" onclick={() => addBlank(group)}>
							{t('scripts.addBlank')}
						</Button>
						<Button size="sm" variant="ghost" icon="file" onclick={() => openTemplates(group)}>
							{t('scripts.addTemplate')}
						</Button>
					</span>
				{/if}
			</div>
			{#each scripts as item, index (item.key)}
				{#if groupOf(item.phase) === group}
					<ScriptCard
						bind:script={scripts[index]}
						number={numbers[item.key] ?? 1}
						{group}
						{secrets}
						{readonly}
						canUp={canMove(scripts, item.key, -1)}
						canDown={canMove(scripts, item.key, 1)}
						error={errorFor(index)}
						onUp={() => (scripts = move(scripts, item.key, -1))}
						onDown={() => (scripts = move(scripts, item.key, 1))}
						onRemove={() => remove(item.key)}
						onNewSecret={(parameter) => openSecret(index, parameter)}
					/>
				{/if}
			{/each}
			{#if (group === 'before' ? beforeCount : afterCount) === 0}
				<p class="list__empty">
					{t(group === 'before' ? 'scripts.emptyBefore' : 'scripts.emptyAfter')}
				</p>
			{/if}
		</section>
	{/each}
</div>

<Dialog
	bind:open={templateOpen}
	size="lg"
	title={t('scripts.template.title')}
	description={t('scripts.template.description')}
>
	<TextField
		label={t('scripts.template.search')}
		placeholder={t('scripts.template.search')}
		bind:value={templateQuery}
	/>
	<ul class="choices" data-testid="template-choices">
		{#each visibleTemplates as template (template.id)}
			<li class="choice">
				<div class="choice__text">
					<span class="choice__name">
						{template.name}
						{#if template.builtin}<Badge tone="accent">{t('scripts.template.builtin')}</Badge>{/if}
					</span>
					<span class="choice__meta">
						{template.description}
						· {t(`scripts.interpreter.${template.interpreter}`)}
						· {t('scripts.template.parameters', { count: template.parameters.length })}
					</span>
				</div>
				<Button onclick={() => addFromTemplate(template)}>{t('scripts.template.use')}</Button>
			</li>
		{:else}
			<li class="choice__meta">{t('scripts.template.empty')}</li>
		{/each}
	</ul>
	{#snippet footer()}
		<Button onclick={() => (templateOpen = false)}>{t('common.cancel')}</Button>
	{/snippet}
</Dialog>

<Dialog
	bind:open={jobTemplateOpen}
	size="lg"
	title={t('scripts.jobTemplate.title')}
	description={t('scripts.jobTemplate.description')}
>
	<ul class="choices" data-testid="job-template-choices">
		{#each jobTemplates as template (template.id)}
			<li class="choice">
				<div class="choice__text">
					<span class="choice__name">
						{template.name}
						{#if template.builtin}<Badge tone="accent">{t('scripts.template.builtin')}</Badge>{/if}
					</span>
					<span class="choice__meta">
						{template.description}
						· {t('scripts.jobTemplate.count', { count: template.scripts.length })}
					</span>
				</div>
				<Button onclick={() => addFromJobTemplate(template)}>{t('scripts.jobTemplate.use')}</Button>
			</li>
		{:else}
			<li class="choice__meta">{t('scripts.jobTemplate.empty')}</li>
		{/each}
	</ul>
	{#snippet footer()}
		<Button onclick={() => (jobTemplateOpen = false)}>{t('common.cancel')}</Button>
	{/snippet}
</Dialog>

<Dialog
	bind:open={secretOpen}
	size="md"
	title={t('scripts.secret.title')}
	description={t('scripts.secret.description')}
	onsubmit={createSecret}
>
	{#if secretError}
		<Alert tone="danger" title={secretError} />
	{/if}
	<TextField
		label={t('scripts.secret.name')}
		error={fieldError(secretProblem, 'name')}
		bind:value={secretName}
		required
	/>
	<PasswordField
		label={t('scripts.secret.value')}
		error={fieldError(secretProblem, 'value')}
		bind:value={secretValue}
		required
	/>
	{#snippet footer()}
		<Button onclick={() => (secretOpen = false)}>{t('common.cancel')}</Button>
		<Button type="submit" variant="primary" loading={secretSaving}>
			{t('scripts.secret.create')}
		</Button>
	{/snippet}
</Dialog>

<style>
	.scripts {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-3);
	}

	.scripts__text {
		margin: 0;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.warn__text {
		margin: 0 0 var(--rh-space-2);
	}

	.scripts__toolbar {
		display: flex;
		gap: var(--rh-space-2);
	}

	.list {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-2);
	}

	.list__head {
		display: flex;
		align-items: center;
		gap: var(--rh-space-2);
	}

	.list__title {
		margin: 0;
		font-size: var(--rh-font-size-sm);
		font-weight: var(--rh-font-weight-semibold);
	}

	.list__actions {
		display: flex;
		gap: var(--rh-space-1);
		margin-inline-start: auto;
	}

	.list__empty {
		margin: 0;
		padding: var(--rh-space-3);
		border: var(--rh-border-width) dashed var(--rh-color-border);
		border-radius: var(--rh-radius-md);
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.choices {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-2);
		margin: var(--rh-space-3) 0 0;
		padding: 0;
		max-height: 360px;
		overflow: auto;
		list-style: none;
	}

	.choice {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: var(--rh-space-3);
		padding: var(--rh-space-2) var(--rh-space-3);
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
	}

	.choice__text {
		display: flex;
		flex-direction: column;
		min-width: 0;
	}

	.choice__name {
		display: inline-flex;
		align-items: center;
		gap: var(--rh-space-2);
		font-weight: var(--rh-font-weight-semibold);
	}

	.choice__meta {
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}
</style>
