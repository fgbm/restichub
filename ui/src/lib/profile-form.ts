/*
 * The model of the profile editor page: the form draft, the conversion to the API shape, the
 * summary of each section, the retention preview and the command preview (FR-PF-01, FR-PF-04).
 */
import type {
	GroupBy,
	JobLimitDefaults,
	ParameterKind,
	Profile,
	ProfileParameter
} from '#lib/api/index.ts';
import { formatMinutes } from '#lib/format.ts';
import { t } from '#lib/i18n/index.ts';
import { pluralRules } from '#lib/intl.ts';
import type { MessageKey } from '#lib/i18n/index.ts';
import { draftOf, newKey, scriptOf } from '#lib/scripts.ts';
import type { ScriptDraft } from '#lib/scripts.ts';
import { usageOf } from '#lib/parameters.ts';
import type { ParameterUsage, UsageSource } from '#lib/parameters.ts';

export type SectionKey = 'excludes' | 'tags' | 'options' | 'retention' | 'check' | 'scripts';
export type RuleKey =
	'keepLast' | 'keepHourly' | 'keepDaily' | 'keepWeekly' | 'keepMonthly' | 'keepYearly';

/** One declared parameter in the form (FR-PF-09). `key` identifies the table row; it is never sent. */
export interface ProfileParameterDraft {
	key: number;
	name: string;
	kind: ParameterKind;
	required: boolean;
	/** A text parameter only: a secret default would give one password to every host (ADR-0047). */
	defaultValue: string;
	description: string;
}

export interface ProfileDraft {
	name: string;
	pathsAll: string;
	pathsWindows: string;
	pathsLinux: string;
	pathsMacos: string;
	excludes: string;
	excludeFiles: string;
	tags: string[];
	vss: boolean;
	oneFileSystem: boolean;
	excludeCaches: boolean;
	limitUpload: string;
	limitDownload: string;
	keepLast: string;
	keepHourly: string;
	keepDaily: string;
	keepWeekly: string;
	keepMonthly: string;
	keepYearly: string;
	keepWithin: string;
	keepTags: string[];
	groupByHost: boolean;
	groupByPaths: boolean;
	groupByTags: boolean;
	filterHosts: string[];
	filterTags: string[];
	filterPaths: string;
	readDataSubset: string;
	stallMinutes: string;
	maxDurationHours: string;
	enabled: Record<SectionKey, boolean>;
	scripts: ScriptDraft[];
	parameters: ProfileParameterDraft[];
}

export const RULES: { key: RuleKey; unit: Unit | 'last'; flag: string }[] = [
	{ key: 'keepLast', unit: 'last', flag: '--keep-last' },
	{ key: 'keepHourly', unit: 'hourly', flag: '--keep-hourly' },
	{ key: 'keepDaily', unit: 'daily', flag: '--keep-daily' },
	{ key: 'keepWeekly', unit: 'weekly', flag: '--keep-weekly' },
	{ key: 'keepMonthly', unit: 'monthly', flag: '--keep-monthly' },
	{ key: 'keepYearly', unit: 'yearly', flag: '--keep-yearly' }
];

type Unit = 'hourly' | 'daily' | 'weekly' | 'monthly' | 'yearly';

export type PresetKey = 'small' | 'standard' | 'long';

export const PRESETS: Record<PresetKey, Record<RuleKey, string>> = {
	small: {
		keepLast: '3',
		keepHourly: '0',
		keepDaily: '7',
		keepWeekly: '0',
		keepMonthly: '0',
		keepYearly: '0'
	},
	standard: {
		keepLast: '3',
		keepHourly: '0',
		keepDaily: '7',
		keepWeekly: '4',
		keepMonthly: '6',
		keepYearly: '0'
	},
	long: {
		keepLast: '3',
		keepHourly: '0',
		keepDaily: '14',
		keepWeekly: '8',
		keepMonthly: '12',
		keepYearly: '5'
	}
};

export function splitLines(value: string): string[] {
	return value
		.split('\n')
		.map((line) => line.trim())
		.filter((line) => line.length > 0);
}

// The limits of the server on the tag lists and the retention lists.
export const MAX_TAGS = 64;
export const MAX_RETENTION_LIST = 64;

export function numberOrZero(value: string): number {
	const parsed = Number.parseInt(value, 10);
	return Number.isFinite(parsed) && parsed > 0 ? parsed : 0;
}

/** An empty field leaves the value to the server. Zero goes to the server, which decides on it. */
function optionalMinutes(value: string): number | undefined {
	const text = String(value ?? '').trim();
	if (text === '') return undefined;
	const parsed = Number.parseInt(text, 10);
	return Number.isFinite(parsed) && parsed > 0 ? parsed : 0;
}

// The form shows hours; the API stores minutes. A value that is not a whole number of minutes rounds to the nearest minute.
function optionalHoursAsMinutes(value: string): number | undefined {
	const text = String(value ?? '')
		.trim()
		.replace(',', '.');
	if (text === '') return undefined;
	const hours = Number(text);
	return Number.isFinite(hours) && hours > 0 ? Math.max(1, Math.round(hours * 60)) : 0;
}

export function minutesToHours(minutes: number): string {
	return String(Math.round((minutes / 60) * 100) / 100);
}

function joinLines(values: string[] | undefined): string {
	return (values ?? []).join('\n');
}

export function emptyDraft(defaults?: JobLimitDefaults): ProfileDraft {
	return {
		name: '',
		pathsAll: '',
		pathsWindows: '',
		pathsLinux: '',
		pathsMacos: '',
		excludes: '',
		excludeFiles: '',
		tags: [],
		vss: false,
		oneFileSystem: false,
		excludeCaches: false,
		limitUpload: '',
		limitDownload: '',
		...PRESETS.standard,
		keepWithin: '',
		keepTags: [],
		groupByHost: false,
		groupByPaths: false,
		groupByTags: false,
		filterHosts: [],
		filterTags: [],
		filterPaths: '',
		readDataSubset: '',
		stallMinutes: defaults ? String(defaults.stall_timeout_minutes) : '',
		maxDurationHours: defaults ? minutesToHours(defaults.max_duration_minutes) : '',
		enabled: {
			excludes: false,
			tags: false,
			options: false,
			retention: true,
			check: false,
			scripts: false
		},
		scripts: [],
		parameters: []
	};
}

/** A new, empty row of the parameter table. The row goes to the request when it has a name. */
export function blankParameter(kind: ParameterKind = 'text'): ProfileParameterDraft {
	return { key: newKey(), name: '', kind, required: false, defaultValue: '', description: '' };
}

export function draftFromProfile(profile: Profile): ProfileDraft {
	const draft = emptyDraft();
	const options = profile.options;
	draft.name = profile.name;
	draft.pathsAll = joinLines(profile.paths?.all);
	draft.pathsWindows = joinLines(profile.paths?.windows);
	draft.pathsLinux = joinLines(profile.paths?.linux);
	draft.pathsMacos = joinLines(profile.paths?.macos);
	draft.excludes = joinLines(profile.excludes);
	draft.excludeFiles = joinLines(profile.exclude_files);
	draft.tags = [...profile.tags];
	draft.vss = options?.use_fs_snapshot ?? false;
	draft.oneFileSystem = options?.one_file_system ?? false;
	draft.excludeCaches = options?.exclude_caches ?? false;
	draft.limitUpload = options?.limit_upload_kib ? String(options.limit_upload_kib) : '';
	draft.limitDownload = options?.limit_download_kib ? String(options.limit_download_kib) : '';
	if (options?.stall_timeout_minutes != null) {
		draft.stallMinutes = String(options.stall_timeout_minutes);
	}
	if (options?.max_duration_minutes != null) {
		draft.maxDurationHours = minutesToHours(options.max_duration_minutes);
	}
	const retention = profile.retention;
	if (retention) {
		draft.keepLast = String(retention.keep_last ?? 0);
		draft.keepHourly = String(retention.keep_hourly ?? 0);
		draft.keepDaily = String(retention.keep_daily ?? 0);
		draft.keepWeekly = String(retention.keep_weekly ?? 0);
		draft.keepMonthly = String(retention.keep_monthly ?? 0);
		draft.keepYearly = String(retention.keep_yearly ?? 0);
		draft.keepWithin = retention.keep_within ?? '';
		draft.keepTags = [...(retention.keep_tags ?? [])];
		const groupBy = retention.group_by ?? [];
		draft.groupByHost = groupBy.includes('host');
		draft.groupByPaths = groupBy.includes('paths');
		draft.groupByTags = groupBy.includes('tags');
		draft.filterHosts = [...(retention.filter?.hosts ?? [])];
		draft.filterTags = [...(retention.filter?.tags ?? [])];
		draft.filterPaths = joinLines(retention.filter?.paths);
	}
	draft.readDataSubset = profile.check?.read_data_subset ?? '';
	draft.scripts = (profile.scripts ?? []).map((script) => draftOf(script));
	draft.parameters = (profile.parameters ?? []).map((parameter) => ({
		key: newKey(),
		name: parameter.name,
		kind: parameter.kind,
		required: parameter.required,
		defaultValue: parameter.default_value ?? '',
		description: parameter.description ?? ''
	}));
	draft.enabled = {
		excludes: draft.excludes !== '' || draft.excludeFiles !== '',
		tags: draft.tags.length > 0,
		options:
			draft.vss ||
			draft.oneFileSystem ||
			draft.excludeCaches ||
			draft.limitUpload !== '' ||
			draft.limitDownload !== '',
		retention: Boolean(retention),
		check: Boolean(profile.check),
		scripts: draft.scripts.length > 0
	};
	return draft;
}

function groupByOf(draft: ProfileDraft): GroupBy[] {
	const keys: GroupBy[] = [];
	if (draft.groupByHost) keys.push('host');
	if (draft.groupByPaths) keys.push('paths');
	if (draft.groupByTags) keys.push('tags');
	return keys;
}

/**
 * The settings that a create or an update request carries. A switched-off section sends its
 * empty value.
 */
export function settingsOf(
	draft: ProfileDraft,
	context: { canEditScripts: boolean; existing: Profile | null }
) {
	const on = draft.enabled;
	return {
		paths: {
			all: splitLines(draft.pathsAll),
			windows: splitLines(draft.pathsWindows),
			linux: splitLines(draft.pathsLinux),
			macos: splitLines(draft.pathsMacos)
		},
		excludes: on.excludes ? splitLines(draft.excludes) : [],
		exclude_files: on.excludes ? splitLines(draft.excludeFiles) : [],
		tags: on.tags ? draft.tags : [],
		options: {
			use_fs_snapshot: on.options && draft.vss,
			one_file_system: on.options && draft.oneFileSystem,
			exclude_caches: on.options && draft.excludeCaches,
			limit_upload_kib: on.options ? numberOrZero(draft.limitUpload) : 0,
			limit_download_kib: on.options ? numberOrZero(draft.limitDownload) : 0,
			stall_timeout_minutes: optionalMinutes(draft.stallMinutes),
			max_duration_minutes: optionalHoursAsMinutes(draft.maxDurationHours)
		},
		// An Operator keeps the scripts as they are stored: only an Admin changes them.
		scripts: context.canEditScripts
			? on.scripts
				? draft.scripts.map(scriptOf)
				: []
			: (context.existing?.scripts ?? []),
		retention: on.retention
			? {
					keep_last: numberOrZero(draft.keepLast),
					keep_hourly: numberOrZero(draft.keepHourly),
					keep_daily: numberOrZero(draft.keepDaily),
					keep_weekly: numberOrZero(draft.keepWeekly),
					keep_monthly: numberOrZero(draft.keepMonthly),
					keep_yearly: numberOrZero(draft.keepYearly),
					keep_within: draft.keepWithin.trim(),
					keep_tags: draft.keepTags,
					group_by: groupByOf(draft),
					filter: {
						hosts: draft.filterHosts,
						tags: draft.filterTags,
						paths: splitLines(draft.filterPaths)
					}
				}
			: null,
		check: on.check ? { read_data_subset: draft.readDataSubset.trim() } : null,
		// A row without a name is a row that the person added and did not fill yet.
		parameters: draft.parameters
			.filter((parameter) => parameter.name.trim() !== '')
			.map((parameter) => ({
				name: parameter.name.trim(),
				kind: parameter.kind,
				required: parameter.required,
				default_value:
					parameter.kind === 'text' && parameter.defaultValue !== ''
						? parameter.defaultValue
						: undefined,
				description: parameter.description.trim() === '' ? undefined : parameter.description.trim()
			}))
	};
}

/** The declarations of the draft as a reader sees them, for the lists that name a parameter. */
export function declaredParameters(draft: ProfileDraft): ProfileParameter[] {
	return draft.parameters
		.filter((parameter) => parameter.name.trim() !== '')
		.map((parameter) => ({
			name: parameter.name.trim(),
			kind: parameter.kind,
			required: parameter.required,
			default_value:
				parameter.kind === 'text' && parameter.defaultValue !== '' ? parameter.defaultValue : null,
			description: parameter.description.trim() === '' ? null : parameter.description.trim(),
			hidden: false
		}));
}

/** The fields of the draft in which a parameter expands, as the switch of each section leaves them. */
export function usageSource(draft: ProfileDraft): UsageSource {
	return {
		paths: [
			...splitLines(draft.pathsAll),
			...splitLines(draft.pathsWindows),
			...splitLines(draft.pathsLinux),
			...splitLines(draft.pathsMacos)
		],
		excludes: draft.enabled.excludes ? splitLines(draft.excludes) : [],
		excludeFiles: draft.enabled.excludes ? splitLines(draft.excludeFiles) : [],
		tags: draft.enabled.tags ? draft.tags : [],
		scripts: draft.enabled.scripts ? draft.scripts : []
	};
}

/** The uses of each declared parameter of the draft, by the name of the parameter. */
export function parameterUses(draft: ProfileDraft): Record<string, ParameterUsage> {
	const source = usageSource(draft);
	return Object.fromEntries(
		draft.parameters
			.filter((parameter) => parameter.name.trim() !== '')
			.map((parameter) => [parameter.name.trim(), usageOf(parameter.name.trim(), source)])
	);
}

/* ------------------------------------------------------------------ plurals */

/** The plural form of a counted word. The catalog holds `.one`, `.few`, `.many` and `.other`. */
export function plural(base: string, count: number): string {
	const category = pluralRules().select(count);
	return t(`${base}.${category}` as MessageKey);
}

/* ---------------------------------------------------------------- retention */

export function hasRetentionRules(draft: ProfileDraft): boolean {
	return (
		RULES.some((rule) => numberOrZero(draft[rule.key]) > 0) ||
		draft.keepWithin.trim() !== '' ||
		draft.keepTags.length > 0
	);
}

export function retentionParts(draft: ProfileDraft): string[] {
	const parts: string[] = [];
	const keys: [RuleKey, MessageKey][] = [
		['keepLast', 'pe.sum.last'],
		['keepHourly', 'pe.sum.hourly'],
		['keepDaily', 'pe.sum.daily'],
		['keepWeekly', 'pe.sum.weekly'],
		['keepMonthly', 'pe.sum.monthly'],
		['keepYearly', 'pe.sum.yearly']
	];
	for (const [key, message] of keys) {
		const count = numberOrZero(draft[key]);
		if (count > 0) parts.push(t(message, { count }));
	}
	if (draft.keepWithin.trim()) parts.push(t('pe.sum.within', { value: draft.keepWithin.trim() }));
	if (draft.keepTags.length > 0) parts.push(t('pe.sum.tags', { tags: draft.keepTags.join(', ') }));
	return parts;
}

export interface SampleSnapshot {
	time: number;
	tags: string[];
}

/** Sixty daily snapshots and two more on each of the last three days. */
export const SAMPLE_SNAPSHOTS: SampleSnapshot[] = (() => {
	const newest = Date.UTC(2026, 9, 15, 2, 0);
	const list: SampleSnapshot[] = [];
	for (let day = 0; day < 60; day++) {
		const time = newest - day * 86_400_000;
		list.push({
			time,
			tags: day === 41 ? ['web', 'nightly', 'before-upgrade'] : ['web', 'nightly']
		});
		if (day < 3) list.push({ time: time + 12 * 3_600_000, tags: ['web', 'nightly'] });
	}
	return list.sort((a, b) => b.time - a.time);
})();

function bucket(unit: Unit, time: number): number {
	const date = new Date(time);
	switch (unit) {
		case 'hourly':
			return Math.floor(time / 3_600_000);
		case 'daily':
			return Math.floor(time / 86_400_000);
		case 'weekly':
			return Math.floor((Math.floor(time / 86_400_000) + 3) / 7);
		case 'monthly':
			return date.getUTCFullYear() * 12 + date.getUTCMonth();
		case 'yearly':
			return date.getUTCFullYear();
	}
}

// restic keeps the newest snapshot of each of the last N intervals that hold a snapshot.
function pickBuckets(unit: Unit, count: number): Set<number> {
	const kept = new Set<number>();
	let left = count;
	let last: number | null = null;
	for (let index = 0; index < SAMPLE_SNAPSHOTS.length && left > 0; index++) {
		const key = bucket(unit, SAMPLE_SNAPSHOTS[index].time);
		if (key !== last) {
			last = key;
			kept.add(index);
			left--;
		}
	}
	return kept;
}

export interface Span {
	y: number;
	m: number;
	d: number;
	h: number;
}

export function parseWithin(value: string): Span | null {
	const text = value.trim();
	if (!text) return null;
	const match = /^(?:(\d+)y)?(?:(\d+)m)?(?:(\d+)d)?(?:(\d+)h)?$/.exec(text);
	if (!match) return null;
	return { y: +(match[1] ?? 0), m: +(match[2] ?? 0), d: +(match[3] ?? 0), h: +(match[4] ?? 0) };
}

function pickWithin(value: string): Set<number> {
	const kept = new Set<number>();
	const span = parseWithin(value);
	if (!span) return kept;
	const latest = new Date(SAMPLE_SNAPSHOTS[0].time);
	const cutoff = Date.UTC(
		latest.getUTCFullYear() - span.y,
		latest.getUTCMonth() - span.m,
		latest.getUTCDate() - span.d,
		latest.getUTCHours() - span.h,
		latest.getUTCMinutes()
	);
	SAMPLE_SNAPSHOTS.forEach((snapshot, index) => {
		if (snapshot.time > cutoff) kept.add(index);
	});
	return kept;
}

function pickTags(tags: string[]): Set<number> {
	const kept = new Set<number>();
	SAMPLE_SNAPSHOTS.forEach((snapshot, index) => {
		if (tags.some((tag) => snapshot.tags.includes(tag))) kept.add(index);
	});
	return kept;
}

export type RuleId = RuleKey | 'keepWithin' | 'keepTags';

export interface RetentionPreview {
	/** The sample snapshots that each rule keeps, by index. */
	perRule: Record<RuleId, Set<number>>;
	kept: Set<number>;
	tagged: Set<number>;
	total: number;
	/** A switched-off policy or a policy without a rule deletes nothing. */
	keepsAll: boolean;
	withinInvalid: boolean;
}

export function previewRetention(draft: ProfileDraft): RetentionPreview {
	const last = numberOrZero(draft.keepLast);
	const perRule: Record<RuleId, Set<number>> = {
		keepLast: new Set(Array.from({ length: Math.min(last, SAMPLE_SNAPSHOTS.length) }, (_, i) => i)),
		keepHourly: pickBuckets('hourly', numberOrZero(draft.keepHourly)),
		keepDaily: pickBuckets('daily', numberOrZero(draft.keepDaily)),
		keepWeekly: pickBuckets('weekly', numberOrZero(draft.keepWeekly)),
		keepMonthly: pickBuckets('monthly', numberOrZero(draft.keepMonthly)),
		keepYearly: pickBuckets('yearly', numberOrZero(draft.keepYearly)),
		keepWithin: pickWithin(draft.keepWithin),
		keepTags: pickTags(draft.keepTags)
	};
	const kept = new Set<number>();
	for (const set of Object.values(perRule)) for (const index of set) kept.add(index);
	return {
		perRule,
		kept,
		tagged: perRule.keepTags,
		total: SAMPLE_SNAPSHOTS.length,
		keepsAll: !draft.enabled.retention || !hasRetentionRules(draft),
		withinInvalid: draft.keepWithin.trim() !== '' && parseWithin(draft.keepWithin) === null
	};
}

/** How many sample snapshots the policy leaves. */
export function keptCount(preview: RetentionPreview): number {
	return preview.keepsAll ? preview.total : preview.kept.size;
}

/* ---------------------------------------------------------------- summaries */

export type SummaryState = 'on' | 'off' | 'warn';

export interface Summary {
	state: SummaryState;
	text: string;
}

export type SummaryKey = 'basics' | 'paths' | SectionKey | 'parameters' | 'limits';

export const SUMMARY_ORDER: SummaryKey[] = [
	'basics',
	'paths',
	'excludes',
	'tags',
	'options',
	'retention',
	'check',
	'scripts',
	'parameters',
	'limits'
];

function linesCount(value: string): number {
	return splitLines(value).length;
}

function switchable(
	draft: ProfileDraft,
	key: SectionKey,
	text: string,
	emptyKey: MessageKey
): Summary {
	if (!draft.enabled[key]) return { state: 'off', text: t('pe.sum.off') };
	if (!text) return { state: 'off', text: t(emptyKey) };
	return { state: 'on', text };
}

function limitText(
	value: number | undefined,
	fallback: number | undefined,
	kind: 'stall' | 'max'
): string {
	if (value === 0 && kind === 'max') return t('pe.sum.maxNone');
	if (value)
		return t(kind === 'stall' ? 'pe.sum.stall' : 'pe.sum.max', { count: formatMinutes(value) });
	if (fallback === undefined)
		return t(kind === 'stall' ? 'pe.sum.stallServer' : 'pe.sum.maxServer');
	return t(kind === 'stall' ? 'pe.sum.stallDefault' : 'pe.sum.maxDefault', {
		count: formatMinutes(fallback)
	});
}

export function summarize(
	draft: ProfileDraft,
	scriptCounts: { before: number; after: number },
	defaults?: JobLimitDefaults
): Record<SummaryKey, Summary> {
	const pathCounts: string[] = [];
	const paths: [string, string][] = [
		[t('pe.sum.pathsAll'), draft.pathsAll],
		['Windows', draft.pathsWindows],
		['Linux', draft.pathsLinux],
		['macOS', draft.pathsMacos]
	];
	for (const [label, value] of paths) {
		if (linesCount(value) > 0) pathCounts.push(`${label}: ${linesCount(value)}`);
	}

	const excludeParts: string[] = [];
	const patterns = linesCount(draft.excludes);
	const files = linesCount(draft.excludeFiles);
	if (patterns > 0) excludeParts.push(`${patterns} ${plural('pe.unit.pattern', patterns)}`);
	if (files > 0) excludeParts.push(`${files} ${plural('pe.unit.excludeFile', files)}`);

	const optionParts: string[] = [];
	if (draft.vss) optionParts.push('VSS');
	if (draft.oneFileSystem) optionParts.push(t('pe.sum.oneFs'));
	if (draft.excludeCaches) optionParts.push(t('pe.sum.caches'));
	if (numberOrZero(draft.limitUpload)) {
		optionParts.push(t('pe.sum.upload', { count: numberOrZero(draft.limitUpload) }));
	}
	if (numberOrZero(draft.limitDownload)) {
		optionParts.push(t('pe.sum.download', { count: numberOrZero(draft.limitDownload) }));
	}

	const retention = retentionParts(draft);
	const retentionSummary: Summary = !draft.enabled.retention
		? { state: 'off', text: t('pe.sum.retentionOff') }
		: retention.length === 0
			? { state: 'warn', text: t('pe.sum.retentionNone') }
			: { state: 'on', text: retention.join(' · ') };

	const subset = draft.readDataSubset.trim();
	const checkSummary: Summary = draft.enabled.check
		? {
				state: 'on',
				text: subset ? t('pe.sum.checkSubset', { value: subset }) : t('pe.sum.checkStructure')
			}
		: { state: 'off', text: t('pe.sum.off') };

	const limits = [
		limitText(optionalMinutes(draft.stallMinutes), defaults?.stall_timeout_minutes, 'stall'),
		limitText(optionalHoursAsMinutes(draft.maxDurationHours), defaults?.max_duration_minutes, 'max')
	];

	const declared = draft.parameters.filter((parameter) => parameter.name.trim() !== '');
	const textCount = declared.filter((parameter) => parameter.kind === 'text').length;
	const secretCount = declared.length - textCount;
	const parameterParts: string[] = [];
	if (declared.length > 0) parameterParts.push(plural('parameters.count', declared.length));
	if (textCount > 0) parameterParts.push(plural('pe.param.summaryText', textCount));
	if (secretCount > 0) parameterParts.push(plural('pe.param.summarySecret', secretCount));

	const scriptTotal = scriptCounts.before + scriptCounts.after;
	return {
		basics: {
			state: draft.name.trim() ? 'on' : 'warn',
			text: draft.name.trim() || t('pe.sum.noName')
		},
		paths:
			pathCounts.length > 0
				? { state: 'on', text: pathCounts.join(' · ') }
				: { state: 'warn', text: t('pe.sum.noPaths') },
		excludes: switchable(draft, 'excludes', excludeParts.join(' · '), 'pe.sum.noExcludes'),
		tags: switchable(draft, 'tags', draft.tags.join(' · '), 'pe.sum.noTags'),
		options: switchable(draft, 'options', optionParts.join(' · '), 'pe.sum.noOptions'),
		retention: retentionSummary,
		check: checkSummary,
		scripts: switchable(
			draft,
			'scripts',
			scriptTotal > 0
				? t('pe.sum.scripts', { before: scriptCounts.before, after: scriptCounts.after })
				: '',
			'pe.sum.noScripts'
		),
		parameters:
			declared.length > 0
				? { state: 'on', text: parameterParts.join(' · ') }
				: { state: 'off', text: t('pe.param.summaryNone') },
		limits: { state: 'on', text: limits.join(' · ') }
	};
}

export function enabledCount(summaries: Record<SummaryKey, Summary>): {
	on: number;
	total: number;
} {
	const states = Object.values(summaries);
	return { on: states.filter((summary) => summary.state === 'on').length, total: states.length };
}
