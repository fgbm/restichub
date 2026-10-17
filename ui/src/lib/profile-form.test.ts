/*
 * The model of the profile editor: tags, summaries, the retention preview, the request and the
 * command preview (FR-PF-01, FR-PF-04, ADR-0039).
 */
import { describe, expect, it } from 'vitest';
import type { Profile } from '#lib/api/index.ts';
import {
	PRESETS,
	buildSteps,
	draftFromProfile,
	emptyDraft,
	enabledCount,
	keptCount,
	parseWithin,
	previewRetention,
	settingsOf,
	summarize
} from './profile-form';

function stored(overrides: Partial<Profile> = {}): Profile {
	return {
		id: 'p_1',
		name: 'web',
		repository_id: 'r_1',
		paths: { all: ['/etc'], windows: ['C:\\Data'], linux: ['/srv'], macos: [] },
		excludes: ['*.tmp'],
		exclude_files: [],
		tags: ['web', 'nightly'],
		options: {
			use_fs_snapshot: true,
			one_file_system: true,
			exclude_caches: false,
			limit_upload_kib: 0,
			limit_download_kib: 0
		},
		scripts: [],
		retention: null,
		check: null,
		created_at: '2026-01-01T00:00:00Z',
		updated_at: '2026-01-01T00:00:00Z',
		...overrides
	} as Profile;
}

function withRules(rules: Partial<ReturnType<typeof emptyDraft>>) {
	return {
		...emptyDraft('r_1'),
		keepLast: '0',
		keepDaily: '0',
		keepWeekly: '0',
		keepMonthly: '0',
		...rules
	};
}

describe('parseWithin', () => {
	it('reads the restic period format', () => {
		expect(parseWithin('1y2m7d3h')).toEqual({ y: 1, m: 2, d: 7, h: 3 });
		expect(parseWithin('30d')).toEqual({ y: 0, m: 0, d: 30, h: 0 });
	});

	it('rejects text that is not a period', () => {
		expect(parseWithin('3 weeks')).toBeNull();
		expect(parseWithin('')).toBeNull();
	});
});

describe('previewRetention', () => {
	it('keeps the N newest snapshots for keep-last', () => {
		const preview = previewRetention(withRules({ keepLast: '3' }));
		expect(keptCount(preview)).toBe(3);
	});

	it('counts days that hold snapshots, one snapshot for each day', () => {
		const preview = previewRetention(withRules({ keepDaily: '7' }));
		expect(keptCount(preview)).toBe(7);
	});

	it('keeps the snapshots of a period counted from the newest snapshot', () => {
		const preview = previewRetention(withRules({ keepWithin: '1d' }));
		expect(keptCount(preview)).toBe(2);
		expect(preview.withinInvalid).toBe(false);
	});

	it('marks a bad period and keeps nothing for it', () => {
		const preview = previewRetention(withRules({ keepWithin: 'soon', keepLast: '1' }));
		expect(preview.withinInvalid).toBe(true);
		expect(keptCount(preview)).toBe(1);
	});

	it('keeps a snapshot with a protected tag', () => {
		const preview = previewRetention(withRules({ keepTags: ['before-upgrade'] }));
		expect(keptCount(preview)).toBe(1);
		expect(preview.tagged.size).toBe(1);
	});

	it('adds the rules together', () => {
		const one = keptCount(previewRetention(withRules({ keepLast: '3' })));
		const both = keptCount(previewRetention(withRules({ keepLast: '3', keepWeekly: '2' })));
		expect(both).toBeGreaterThan(one);
	});

	it('keeps all when no rule is set or the policy is off', () => {
		const none = previewRetention(withRules({}));
		expect(none.keepsAll).toBe(true);
		expect(keptCount(none)).toBe(none.total);
		const off = previewRetention({
			...withRules({ keepLast: '3' }),
			enabled: { ...emptyDraft().enabled, retention: false }
		});
		expect(keptCount(off)).toBe(off.total);
	});

	it('has the standard preset keep more than the minimum preset', () => {
		const small = keptCount(previewRetention(withRules(PRESETS.small)));
		const standard = keptCount(previewRetention(withRules(PRESETS.standard)));
		expect(standard).toBeGreaterThan(small);
	});
});

describe('summarize', () => {
	const none = { before: 0, after: 0 };

	it('describes a new profile with its warnings', () => {
		const sums = summarize(emptyDraft(), '', none);
		expect(sums.basics.state).toBe('warn');
		expect(sums.paths).toEqual({ state: 'warn', text: 'No paths set' });
		expect(sums.excludes).toEqual({ state: 'off', text: 'Off' });
		expect(sums.limits.text).toBe('no progress: server default · longest time: server default');
		expect(
			summarize(emptyDraft(), '', none, { stall_timeout_minutes: 30, max_duration_minutes: 90 })
				.limits.text
		).toBe('no progress 30m (default) · at most 1h 30m (default)');
	});

	it('sums up a stored profile', () => {
		const draft = draftFromProfile(
			stored({
				retention: { keep_last: 3, keep_daily: 7, keep_within: '30d', keep_tags: ['keep'] },
				check: { read_data_subset: '5%' }
			})
		);
		const sums = summarize(draft, 'offsite', { before: 1, after: 2 });
		expect(sums.basics.text).toBe('web · offsite');
		expect(sums.paths.text).toBe('all systems: 1 · Windows: 1 · Linux: 1');
		expect(sums.excludes.text).toBe('1 pattern');
		expect(sums.tags.text).toBe('web · nightly');
		expect(sums.options.text).toBe('VSS · stay on one file system');
		expect(sums.retention.text).toBe('last 3 · by day 7 · all for 30d · tags: keep');
		expect(sums.check.text).toBe('read 5% of the data');
		expect(sums.scripts.state).toBe('off');
	});

	it('warns when retention is on without a rule', () => {
		const draft = withRules({});
		expect(summarize(draft, 'r', none).retention).toEqual({
			state: 'warn',
			text: 'No rules: nothing is deleted'
		});
	});

	it('counts the sections that are on', () => {
		const draft = draftFromProfile(stored());
		expect(enabledCount(summarize(draft, 'r', none))).toEqual({ on: 6, total: 9 });
	});
});

describe('settingsOf', () => {
	const context = { canEditScripts: true, existing: null };

	it('sends the empty value of a switched-off section', () => {
		const draft = { ...draftFromProfile(stored()) };
		draft.enabled = { ...draft.enabled, tags: false, options: false };
		const settings = settingsOf(draft, context);
		expect(settings.tags).toEqual([]);
		expect(settings.options.use_fs_snapshot).toBe(false);
		expect(settings.retention).toBeNull();
		expect(settings.check).toBeNull();
	});

	it('omits an empty limit field and sends the entered minutes', () => {
		const draft = emptyDraft('r_1');
		const empty = settingsOf(draft, context).options;
		expect(empty.stall_timeout_minutes).toBeUndefined();
		expect(empty.max_duration_minutes).toBeUndefined();
		draft.stallMinutes = '15';
		draft.maxDurationHours = '3';
		expect(settingsOf(draft, context).options).toMatchObject({
			stall_timeout_minutes: 15,
			max_duration_minutes: 180
		});
		draft.stallMinutes = '0';
		expect(settingsOf(draft, context).options.stall_timeout_minutes).toBe(0);
		draft.maxDurationHours = '1.5';
		expect(settingsOf(draft, context).options.max_duration_minutes).toBe(90);
		draft.maxDurationHours = '0';
		expect(settingsOf(draft, context).options.max_duration_minutes).toBe(0);
	});

	it('keeps the stored scripts of an Operator', () => {
		const existing = stored({
			scripts: [{ name: 'S' } as never]
		});
		const settings = settingsOf(draftFromProfile(existing), {
			canEditScripts: false,
			existing
		});
		expect(settings.scripts).toEqual(existing.scripts);
	});

	it('round-trips a retention policy', () => {
		const retention = {
			keep_last: 3,
			keep_hourly: 0,
			keep_daily: 7,
			keep_weekly: 4,
			keep_monthly: 0,
			keep_yearly: 0,
			keep_within: '',
			keep_tags: ['keep'],
			group_by: ['host' as const],
			filter: { hosts: ['web-*'], tags: ['db'], paths: [] }
		};
		const existing = stored({ retention });
		expect(
			settingsOf(draftFromProfile(existing), { canEditScripts: true, existing }).retention
		).toEqual(retention);
	});

	it('fills a new draft with the server defaults and shows the stored limits', () => {
		const defaults = { stall_timeout_minutes: 60, max_duration_minutes: 2880 };
		const fresh = emptyDraft('r_1', defaults);
		expect(fresh.stallMinutes).toBe('60');
		expect(fresh.maxDurationHours).toBe('48');
		expect(emptyDraft('r_1').stallMinutes).toBe('');
		const draft = draftFromProfile(
			stored({
				options: { stall_timeout_minutes: 30, max_duration_minutes: 0 } as Profile['options']
			})
		);
		expect(draft.stallMinutes).toBe('30');
		expect(draft.maxDurationHours).toBe('0');
	});
});

describe('buildSteps', () => {
	const scripts = [
		{ name: 'Stop db', phase: 'before' as const },
		{ name: 'Start db', phase: 'after' as const }
	];

	it('takes the paths of the chosen system', () => {
		const draft = draftFromProfile(stored());
		const linux = buildSteps(draft, 'linux', []).find((step) => step.kind === 'backup');
		const windows = buildSteps(draft, 'windows', []).find((step) => step.kind === 'backup');
		expect(linux?.command).toContain('/srv');
		expect(linux?.command).not.toContain('C:\\Data');
		expect(linux?.command).not.toContain('--use-fs-snapshot');
		expect(windows?.command).toContain('C:\\Data');
		expect(windows?.command).toContain('--use-fs-snapshot');
	});

	it('adds forget only when a rule is set and orders the script steps', () => {
		const withRetention = draftFromProfile(
			stored({ retention: { keep_daily: 7 }, scripts: [{ name: 'x' } as never] })
		);
		withRetention.enabled.scripts = true;
		const kinds = buildSteps(withRetention, 'linux', scripts).map((step) => step.kind);
		expect(kinds).toEqual(['script', 'backup', 'script', 'forget']);
		const noRule = draftFromProfile(stored({ retention: { keep_daily: 0 } }));
		expect(buildSteps(noRule, 'linux', []).map((step) => step.kind)).toEqual(['backup']);
	});

	it('quotes a value that holds a space or a star', () => {
		const draft = draftFromProfile(stored({ excludes: ['*.tmp', 'my dir'] }));
		const command = buildSteps(draft, 'linux', [])[0].command;
		expect(command).toContain("--exclude '*.tmp'");
		expect(command).toContain("--exclude 'my dir'");
	});
});
