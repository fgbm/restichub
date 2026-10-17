<script lang="ts">
	import type { ApiProblem } from '#lib/api/index.ts';
	import { fieldError } from '#lib/api/index.ts';
	import { Button, Checkbox, Disclosure, Field, TextField } from './ui/index.ts';
	import RichText from './RichText.svelte';
	import TagInput from './ui/TagInput.svelte';
	import { t } from '#lib/i18n/index.ts';
	import type { MessageKey } from '#lib/i18n/index.ts';
	import {
		MAX_RETENTION_LIST,
		PRESETS,
		RULES,
		SAMPLE_SNAPSHOTS,
		keptCount,
		plural,
		previewRetention
	} from '#lib/profile-form.ts';
	import type { PresetKey, ProfileDraft, RuleId } from '#lib/profile-form.ts';

	let {
		draft = $bindable(),
		problem = null
	}: { draft: ProfileDraft; problem?: ApiProblem | null } = $props();

	const preview = $derived(previewRetention(draft));
	const kept = $derived(keptCount(preview));
	const presetKeys: PresetKey[] = ['small', 'standard', 'long'];
	const presetLabels: Record<PresetKey, MessageKey> = {
		small: 'pe.retention.preset.small',
		standard: 'pe.retention.preset.standard',
		long: 'pe.retention.preset.long'
	};

	function applyPreset(key: PresetKey): void {
		Object.assign(draft, PRESETS[key], { keepWithin: '' });
	}

	function titleOf(rule: string): MessageKey {
		return `pe.rule.${rule}.title` as MessageKey;
	}

	function countOf(id: RuleId): number {
		return preview.perRule[id].size;
	}

	function snapshotLabel(time: number, isKept: boolean): string {
		const date = new Date(time);
		const pad = (value: number) => String(value).padStart(2, '0');
		const when = `${pad(date.getUTCDate())}.${pad(date.getUTCMonth() + 1)} ${pad(date.getUTCHours())}:00`;
		return `${when} — ${t(isKept ? 'pe.timeline.kept' : 'pe.timeline.deleted')}`;
	}

	const filterSummary = $derived.by(() => {
		const parts: string[] = [];
		const hosts = draft.filterHosts.length;
		const paths = draft.filterPaths.split('\n').filter((line) => line.trim()).length;
		if (hosts) parts.push(t('pe.filter.hosts', { count: hosts }));
		if (draft.filterTags.length)
			parts.push(t('pe.filter.tags', { count: draft.filterTags.length }));
		if (paths) parts.push(t('pe.filter.paths', { count: paths }));
		return parts.length > 0 ? parts.join(', ') : t('pe.filter.all');
	});
</script>

<div class="howto">
	<div class="howto__title">{t('pe.retention.howto.title')}</div>
	<ul>
		<li><RichText text={t('pe.retention.howto.union')} /></li>
		<li><RichText text={t('pe.retention.howto.zero')} /></li>
		<li><RichText text={t('pe.retention.howto.intervals')} /></li>
	</ul>
</div>

<div class="presets">
	<span class="muted">{t('pe.retention.presets')}</span>
	{#each presetKeys as key (key)}
		<Button size="sm" onclick={() => applyPreset(key)}>{t(presetLabels[key])}</Button>
	{/each}
</div>

<div class="rh-field">
	<span class="rh-field__label" id="profile-rules-label">{t('pe.rules.title')}</span>
	<div class="rules" role="group" aria-labelledby="profile-rules-label">
		{#each RULES as rule (rule.key)}
			<div class="rule">
				<div class="rule__head">
					<span class="rule__title">{t(titleOf(rule.unit))}</span>
					<code class="flag">{rule.flag}</code>
				</div>
				<div class="rule__input">
					<TextField
						id={`profile-${rule.key.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`)}`}
						type="number"
						min="0"
						aria-label={t(titleOf(rule.unit))}
						bind:value={draft[rule.key]}
					/>
					<span class="muted">{t(`pe.rule.${rule.unit}.unit` as MessageKey)}</span>
				</div>
				<p class="rule__text">{t(`pe.rule.${rule.unit}.text` as MessageKey)}</p>
				<div class="rule__count">
					{#if countOf(rule.key) > 0}
						<RichText text={t('pe.rule.count', { count: countOf(rule.key) })} />
					{:else}
						<span class="muted">{t('pe.rule.off')}</span>
					{/if}
				</div>
			</div>
		{/each}
		<div class="rule">
			<div class="rule__head">
				<span class="rule__title">{t('pe.rule.within.title')}</span>
				<code class="flag">--keep-within</code>
			</div>
			<div class="rule__input">
				<TextField
					id="profile-keep-within"
					mono
					placeholder={t('common.example', { value: '30d' })}
					aria-label={t('pe.rule.within.title')}
					error={preview.withinInvalid
						? t('pe.rule.within.invalid')
						: fieldError(problem, 'retention.keep_within')}
					bind:value={draft.keepWithin}
				/>
				<span class="muted rule__hint">{t('pe.rule.within.hint')}</span>
			</div>
			<p class="rule__text"><RichText text={t('pe.rule.within.text')} /></p>
			<div class="rule__count">
				{#if countOf('keepWithin') > 0}
					<RichText text={t('pe.rule.count', { count: countOf('keepWithin') })} />
				{:else}
					<span class="muted">{t('pe.rule.off')}</span>
				{/if}
			</div>
		</div>
	</div>
</div>

<div class="rh-field">
	<span class="rh-field__label">
		{t('pe.rule.tags.title')}
		<code class="flag">--keep-tag</code>
	</span>
	<TagInput
		id="profile-keep-tags"
		label={t('profiles.tags')}
		placeholder={t('pe.tags.placeholder')}
		max={MAX_RETENTION_LIST}
		error={fieldError(problem, 'retention.keep_tags')}
		bind:value={draft.keepTags}
	/>
	<p class="rh-field__help">{t('pe.rule.tags.text')}</p>
	<div class="rule__count">
		{#if countOf('keepTags') > 0}
			<RichText text={t('pe.rule.count', { count: countOf('keepTags') })} />
		{:else}
			<span class="muted">{t('pe.rule.off')}</span>
		{/if}
	</div>
</div>

<div class="rh-field">
	<span class="rh-field__label">
		{t('pe.rule.group.title')}
		<code class="flag">--group-by</code>
	</span>
	<div class="checks">
		<Checkbox label={t('profiles.retention.groupBy.host')} bind:checked={draft.groupByHost} />
		<Checkbox label={t('profiles.retention.groupBy.paths')} bind:checked={draft.groupByPaths} />
		<Checkbox label={t('profiles.retention.groupBy.tags')} bind:checked={draft.groupByTags} />
	</div>
	<p class="rh-field__help">{t('pe.rule.group.text')}</p>
</div>

<Disclosure label={t('pe.filter.title')} hint={filterSummary}>
	<div class="filter__body">
		<p class="muted">{t('pe.filter.hint')}</p>
		<div class="grid3">
			<TagInput
				id="profile-filter-hosts"
				label={t('profiles.retention.filterHosts')}
				mono
				max={MAX_RETENTION_LIST}
				error={fieldError(problem, 'retention.filter.hosts')}
				bind:value={draft.filterHosts}
			/>
			<TagInput
				id="profile-filter-tags"
				label={t('profiles.retention.filterTags')}
				placeholder={t('pe.tags.placeholder')}
				max={MAX_RETENTION_LIST}
				error={fieldError(problem, 'retention.filter.tags')}
				bind:value={draft.filterTags}
			/>
			<Field
				label={t('profiles.retention.filterPaths')}
				help={t('pe.filter.oneEach')}
				error={fieldError(problem, 'retention.filter.paths')}
				for="profile-filter-paths"
			>
				<textarea
					id="profile-filter-paths"
					class="rh-textarea rh-input--mono"
					rows="2"
					bind:value={draft.filterPaths}></textarea>
			</Field>
		</div>
	</div>
</Disclosure>

<div class="preview">
	<div class="preview__head">
		<span class="group-title">{t('pe.timeline.title')}</span>
		<span class="muted">{t('pe.timeline.sample')}</span>
	</div>
	<div class="timeline" role="img" aria-label={t('pe.timeline.label')}>
		{#each [...SAMPLE_SNAPSHOTS].reverse() as snapshot, position (snapshot.time)}
			{@const index = SAMPLE_SNAPSHOTS.length - 1 - position}
			{@const isKept = preview.keepsAll || preview.kept.has(index)}
			<i
				class="snap"
				class:snap--kept={isKept}
				class:snap--tagged={!preview.keepsAll && preview.tagged.has(index)}
				title={snapshotLabel(snapshot.time, isKept)}
			></i>
		{/each}
	</div>
	<div class="axis">
		<span>{t('pe.timeline.axisStart')}</span><span>{t('pe.timeline.axisMiddle')}</span><span
			>{t('pe.timeline.axisEnd')}</span
		>
	</div>
	<div class="legend">
		<span><i class="snap snap--kept"></i> {t('pe.timeline.kept')}</span>
		<span><i class="snap"></i> {t('pe.timeline.deleted')}</span>
		<span><i class="snap snap--kept snap--tagged"></i> {t('pe.timeline.tagged')}</span>
	</div>
	<p class="keepline" data-testid="keep-line">
		{#if !draft.enabled.retention}
			<RichText text={t('pe.timeline.off', { total: preview.total })} />
		{:else if preview.keepsAll}
			<RichText text={t('pe.timeline.noRules', { total: preview.total })} />
		{:else}
			<RichText
				text={t('pe.timeline.result', {
					count: kept,
					noun: plural('pe.unit.snapshot', kept),
					total: preview.total,
					deleted: preview.total - kept
				})}
			/>
		{/if}
	</p>
</div>

<style>
	.muted {
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.howto {
		padding: var(--rh-space-3) var(--rh-space-4);
		border: var(--rh-border-width) solid var(--rh-color-info-subtle-border);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-info-subtle);
		color: var(--rh-color-info-subtle-text);
		font-size: var(--rh-font-size-sm);
	}

	.howto__title {
		font-weight: var(--rh-font-weight-semibold);
	}

	.howto ul {
		margin: var(--rh-space-1) 0 0;
		padding-left: var(--rh-space-4);
	}

	.presets {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: var(--rh-space-2);
	}

	.rules {
		display: flex;
		flex-direction: column;
		border-top: var(--rh-border-width) solid var(--rh-color-border-muted);
	}

	.rule {
		display: grid;
		grid-template-columns: minmax(0, 1fr) 14rem;
		gap: var(--rh-space-1) var(--rh-space-4);
		padding: var(--rh-space-3) 0;
		border-bottom: var(--rh-border-width) solid var(--rh-color-border-muted);
	}

	.rule__head {
		display: flex;
		align-items: center;
		gap: var(--rh-space-2);
	}

	.rule__title {
		font-weight: var(--rh-font-weight-semibold);
	}

	.flag {
		padding: 0 var(--rh-space-1);
		border-radius: var(--rh-radius-sm);
		background-color: var(--rh-color-code-bg);
		font-family: var(--rh-font-mono);
		font-size: var(--rh-font-size-xs);
	}

	.rule__input {
		display: grid;
		grid-column: 2;
		grid-row: 1;
		grid-template-columns: 6rem minmax(0, 1fr);
		align-items: center;
		gap: var(--rh-space-1) var(--rh-space-2);
	}

	.rule__input :global(.rh-input) {
		width: 100%;
	}

	.rule__hint {
		grid-column: 1 / -1;
	}

	.rule__text {
		grid-column: 1;
		margin: 0;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.rule__count {
		grid-column: 2;
		grid-row: 2;
		font-size: var(--rh-font-size-sm);
	}

	.checks {
		display: flex;
		flex-wrap: wrap;
		gap: var(--rh-space-4);
	}

	.filter__body {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-3);
	}

	.grid3 {
		display: grid;
		grid-template-columns: repeat(3, minmax(0, 1fr));
		gap: var(--rh-space-3);
	}

	.preview {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-2);
		padding: var(--rh-space-3) var(--rh-space-4);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-surface-muted);
	}

	.preview__head {
		display: flex;
		flex-wrap: wrap;
		align-items: baseline;
		gap: var(--rh-space-3);
	}

	.group-title {
		font-weight: var(--rh-font-weight-semibold);
	}

	.timeline {
		display: flex;
		align-items: flex-end;
		gap: 2px;
		height: 36px;
	}

	.snap {
		display: inline-block;
		flex: 1;
		height: 10px;
		border-radius: 1px;
		background-color: var(--rh-color-border-strong);
		opacity: 0.55;
	}

	.timeline .snap--kept,
	.legend .snap--kept {
		height: 28px;
		background-color: var(--rh-color-success);
		opacity: 1;
	}

	.timeline .snap--tagged,
	.legend .snap--tagged {
		background-color: var(--rh-color-info);
	}

	.legend .snap {
		flex: none;
		width: 8px;
		height: 12px;
		vertical-align: middle;
	}

	.axis {
		display: flex;
		justify-content: space-between;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-xs);
	}

	.legend {
		display: flex;
		flex-wrap: wrap;
		gap: var(--rh-space-4);
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-xs);
	}

	.keepline {
		margin: 0;
	}

	@media (max-width: 720px) {
		.grid3 {
			grid-template-columns: minmax(0, 1fr);
		}
	}
</style>
