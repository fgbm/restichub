<script lang="ts">
	import { onMount } from 'svelte';
	import { beforeNavigate, goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { ApiError, fieldError, service } from '#lib/api/index.ts';
	import type {
		ApiProblem,
		HookPhase,
		JobTemplate,
		ScriptTemplate,
		Secret
	} from '#lib/api/index.ts';
	import ProfileSection from './ProfileSection.svelte';
	import ScriptsEditor from './ScriptsEditor.svelte';
	import { RecordPage, SummaryPanel } from './layout/index.ts';
	import { Alert, Button, ConfirmDialog, Icon, TextField } from './ui/index.ts';
	import type { KeyValueItem } from './ui/index.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import { changedAt, plural } from '#lib/list-format.ts';
	import { draftOf, scriptOf, templateDescription, templateName } from '#lib/scripts.ts';
	import type { ScriptDraft } from '#lib/scripts.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';
	import { interpreterLabel } from '#lib/template-item.ts';

	let {
		templateId = null,
		copyFrom = null
	}: {
		templateId?: string | null;
		/** A new template starts as a copy of this template. */
		copyFrom?: string | null;
	} = $props();

	type SectionKey = 'basics' | 'scripts';

	let loading = $state(true);
	let loadError = $state<string | null>(null);
	let existing = $state<JobTemplate | null>(null);
	let scriptTemplates = $state<ScriptTemplate[]>([]);
	let secrets = $state<Secret[]>([]);
	let saving = $state(false);
	let saved = false;
	let formError = $state<string | null>(null);
	let formProblem = $state<ApiProblem | null>(null);
	let baseline = $state('');
	let open = $state<Record<SectionKey, boolean>>({ basics: true, scripts: true });

	let name = $state('');
	let description = $state('');
	let scripts = $state<ScriptDraft[]>([]);

	let deleteOpen = $state(false);
	let deleting = $state(false);

	const canWrite = $derived(getSession()?.role === 'admin');
	// A built-in template is read-only: a copy changes it.
	const readonly = $derived(Boolean(existing?.builtin) || !canWrite);

	function fill(template: JobTemplate): void {
		name = templateName(template, 'job');
		description = templateDescription(template, 'job');
		scripts = template.scripts.map((script) => draftOf(script));
	}

	function snapshot(): string {
		return JSON.stringify([name, description, scripts.map(scriptOf)]);
	}

	const dirty = $derived(!loading && !readonly && baseline !== '' && snapshot() !== baseline);

	function messageOf(problem: unknown): string {
		if (problem instanceof ApiError) return tErrorCode(problem.code) ?? problem.title;
		return t('table.error');
	}

	function problemOf(problem: unknown): ApiProblem | null {
		if (problem instanceof ApiError) return problem.problem;
		if (problem && typeof problem === 'object' && 'problem' in problem) {
			return (problem as { problem: ApiProblem }).problem;
		}
		return null;
	}

	async function load(): Promise<void> {
		try {
			const templates = await service.listJobTemplates();
			if (templateId) {
				const found = templates.find((item) => item.id === templateId);
				if (!found) {
					loadError = t('jobTemplates.notFound');
					return;
				}
				existing = found;
				fill(found);
			} else {
				const source = copyFrom ? templates.find((item) => item.id === copyFrom) : undefined;
				if (source) {
					fill(source);
					name = t('scriptTemplates.copyName', { name: templateName(source, 'job') });
				}
			}
			// The script templates and the secrets serve the editor only; a failure leaves it without
			// choices and does not block the page.
			const [scriptList, secretList] = await Promise.allSettled([
				service.listScriptTemplates(),
				service.listSecrets()
			]);
			scriptTemplates = scriptList.status === 'fulfilled' ? scriptList.value : [];
			secrets = secretList.status === 'fulfilled' ? secretList.value : [];
			baseline = snapshot();
		} catch (problem) {
			loadError = messageOf(problem);
		} finally {
			loading = false;
		}
	}

	onMount(load);

	beforeNavigate(({ cancel }) => {
		if (dirty && !saved && !window.confirm(t('rp.unsaved'))) cancel();
	});

	function toList(): void {
		void goto(resolve('/(app)/settings/scripts/jobs'));
	}

	function openCopy(): void {
		if (!existing) return;
		const target = `${resolve('/(app)/settings/scripts/jobs/new')}?from=${encodeURIComponent(existing.id)}`;
		// The address is the resolved path of the new template page with a query.
		// eslint-disable-next-line svelte/no-navigation-without-resolve
		void goto(target);
	}

	async function save(): Promise<void> {
		if (readonly) return;
		saving = true;
		formError = null;
		formProblem = null;
		try {
			const shared = {
				name: name.trim(),
				description: description.trim(),
				scripts: scripts.map(scriptOf)
			};
			if (existing) await service.updateJobTemplate(existing.id, shared);
			else await service.createJobTemplate(shared);
			pushToast('success', t(existing ? 'jobTemplates.updated' : 'jobTemplates.created'));
			saved = true;
			toList();
		} catch (problem) {
			formProblem = problemOf(problem);
			formError = formProblem?.field ? null : messageOf(problem);
		} finally {
			saving = false;
		}
	}

	async function confirmDelete(): Promise<void> {
		if (!existing) return;
		deleting = true;
		try {
			await service.deleteJobTemplate(existing.id);
			pushToast('success', t('jobTemplates.deleted'));
			deleteOpen = false;
			saved = true;
			toList();
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		} finally {
			deleting = false;
		}
	}

	// The run order of the phases around the backup (FR-HK-12).
	const BEFORE: HookPhase[] = ['before'];
	const AFTER: HookPhase[] = ['after', 'on_success', 'on_error', 'finally'];

	interface Step {
		phase: HookPhase;
		names: string[];
	}

	function stepsOf(phases: HookPhase[]): Step[] {
		return phases
			.map((phase) => ({
				phase,
				names: scripts.filter((script) => script.phase === phase).map((script) => script.name)
			}))
			.filter((step) => step.names.length > 0);
	}

	const before = $derived(stepsOf(BEFORE));
	const after = $derived(stepsOf(AFTER));

	const secretCount = $derived(
		scripts.reduce(
			(count, script) => count + script.env.filter((item) => item.kind === 'secret_ref').length,
			0
		)
	);

	const summaryItems = $derived<KeyValueItem[]>([
		{
			key: 'source',
			label: t('scriptTemplates.source'),
			value: existing?.builtin ? t('scriptTemplates.builtin') : t('scriptTemplates.own')
		},
		{ key: 'scripts', label: t('jobTemplates.scriptCount'), value: String(scripts.length) },
		...(scripts.length > 0
			? [
					{
						key: 'interpreters',
						label: t('jobTemplates.interpreters'),
						value: [...new Set(scripts.map((script) => script.interpreter))]
							.map(interpreterLabel)
							.join(', ')
					}
				]
			: []),
		...(secretCount > 0
			? [{ key: 'secrets', label: t('jobTemplates.secrets'), value: String(secretCount) }]
			: []),
		...(existing && !existing.builtin && (existing.updated_at ?? existing.created_at)
			? [
					{
						key: 'changed',
						label: t('templates.changed'),
						value: changedAt(existing.updated_at ?? existing.created_at ?? '')
					}
				]
			: [])
	]);

	const sectionSummaries = $derived({
		basics: name.trim() || t('templates.unnamed'),
		scripts:
			scripts.length > 0
				? plural('templates.scriptCount', scripts.length)
				: t('templates.meta.noScripts')
	});
</script>

{#snippet step(item: Step)}
	<li class="order__step">
		<span class="order__phase">{t(`profiles.hooks.phase.${item.phase}`)}</span>
		<span class="order__names">
			{#each item.names as scriptName, index (index)}
				<span>{scriptName || t('templates.unnamed')}</span>
			{/each}
		</span>
	</li>
{/snippet}

<RecordPage
	title={templateId
		? name || (existing ? templateName(existing, 'job') : '')
		: t('jobTemplates.newTitle')}
	description={t('jobTemplates.formDescription')}
	{loading}
	error={loadError}
	onsubmit={() => void save()}
	save={readonly ? undefined : { dirty, saving, create: !templateId, oncancel: toList }}
>
	{#snippet actions()}
		{#if existing && canWrite}
			<Button variant={existing.builtin ? 'primary' : 'secondary'} icon="copy" onclick={openCopy}>
				{t('jobTemplates.copy')}
			</Button>
			{#if !existing.builtin}
				<Button
					variant="ghost"
					icon="trash"
					class="btn--quiet-danger"
					title={t('scriptTemplates.delete')}
					aria-label={t('scriptTemplates.delete')}
					onclick={() => (deleteOpen = true)}
				/>
			{/if}
		{/if}
	{/snippet}
	{#snippet banners()}
		{#if existing?.builtin}
			<Alert variant="inline" icon="lock" title={t('scriptTemplates.builtinNote')} />
		{/if}
		{#if formError}
			<Alert tone="danger" title={formError} />
		{/if}
	{/snippet}
	{#snippet side()}
		<SummaryPanel title={t('jobTemplates.order')}>
			{#if scripts.length === 0}
				<p class="muted">{t('templates.meta.noScripts')}</p>
			{:else}
				<ol class="order">
					{#each before as item (item.phase)}
						{@render step(item)}
					{/each}
					<li class="order__step order__step--backup">
						<Icon name="repository" size="xs" />
						{t('jobTemplates.backupStep')}
					</li>
					{#each after as item (item.phase)}
						{@render step(item)}
					{/each}
				</ol>
			{/if}
		</SummaryPanel>
		<SummaryPanel title={t('templates.summary')} items={summaryItems} />
	{/snippet}

	<ProfileSection
		id="basics"
		title={t('scriptTemplates.section.basics')}
		summary={sectionSummaries.basics}
		state="on"
		required
		bind:open={open.basics}
	>
		<TextField
			id="job-template-name"
			label={t('jobTemplates.name')}
			error={fieldError(formProblem, 'name')}
			bind:value={name}
			{readonly}
			required
		/>
		<TextField
			id="job-template-description"
			label={t('scriptTemplates.description.label')}
			error={fieldError(formProblem, 'description')}
			bind:value={description}
			{readonly}
		/>
	</ProfileSection>

	<ProfileSection
		id="scripts"
		title={t('jobTemplates.scripts')}
		summary={sectionSummaries.scripts}
		state="on"
		bind:open={open.scripts}
	>
		<ScriptsEditor
			bind:scripts
			bind:secrets
			templates={scriptTemplates}
			jobTemplates={[]}
			{readonly}
			problem={formProblem}
			framed={false}
		/>
	</ProfileSection>
</RecordPage>

<ConfirmDialog
	bind:open={deleteOpen}
	title={t('jobTemplates.deleteTitle')}
	message={t('jobTemplates.deleteBody', {
		name: existing ? templateName(existing, 'job') : ''
	})}
	confirmLabel={t('scriptTemplates.delete')}
	tone="danger"
	loading={deleting}
	onConfirm={confirmDelete}
/>

<style>
	.muted {
		margin: 0;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.order {
		display: flex;
		flex-direction: column;
		margin: 0;
		padding: 0;
		list-style: none;
		font-size: var(--rh-font-size-sm);
	}

	.order__step {
		display: flex;
		gap: var(--rh-space-2);
		padding: 6px 0 6px var(--rh-space-3);
		border-left: 2px solid var(--rh-color-border);
	}

	.order__step--backup {
		align-items: center;
		color: var(--rh-color-text-muted);
	}

	.order__phase {
		flex: none;
		width: 76px;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-xs);
	}

	.order__names {
		display: flex;
		flex-direction: column;
		min-width: 0;
		overflow-wrap: anywhere;
	}
</style>
