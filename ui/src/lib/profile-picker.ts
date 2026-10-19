import type { Profile } from '#lib/api/index.ts';
import type { FilterOption, RecordOption, RecordPart } from '#lib/components/ui/index.ts';
import { t } from '#lib/i18n/index.ts';
import type { MessageKey } from '#lib/i18n/index.ts';
import { parameterChip } from '#lib/parameters.ts';

type ProfileRef = Pick<Profile, 'id' | 'name' | 'paths' | 'retention'> & {
	parameters?: Profile['parameters'];
};

const RETENTION: [keyof NonNullable<Profile['retention']>, MessageKey][] = [
	['keep_last', 'profiles.short.last'],
	['keep_hourly', 'profiles.short.hourly'],
	['keep_daily', 'profiles.short.daily'],
	['keep_weekly', 'profiles.short.weekly'],
	['keep_monthly', 'profiles.short.monthly'],
	['keep_yearly', 'profiles.short.yearly']
];

/** Every path of a profile, in the order that the editor shows them. */
export function profilePathList(profile: ProfileRef): string[] {
	const paths = profile.paths;
	return [
		...(paths?.all ?? []),
		...(paths?.windows ?? []),
		...(paths?.linux ?? []),
		...(paths?.macos ?? [])
	];
}

/** The first path of a profile and the count of its paths, or null when it has none. */
export function profilePaths(profile: ProfileRef): { first: string; total: number } | null {
	const all = profilePathList(profile);
	return all.length > 0 ? { first: all[0], total: all.length } : null;
}

/** The first path with the count of the others, for example "/srv +2". */
export function profilePathText(profile: ProfileRef): string {
	const path = profilePaths(profile);
	if (!path) return '';
	return path.total > 1 ? `${path.first} +${path.total - 1}` : path.first;
}

/** The retention rules in one short line, for example "7 d · 2 wk · 4 mo". */
export function profileRetention(profile: ProfileRef): string {
	const retention = profile.retention;
	if (!retention) return '';
	const parts = RETENTION.filter(([key]) => Number(retention[key] ?? 0) > 0).map(([key, message]) =>
		t(message, { count: Number(retention[key]) })
	);
	if (retention.keep_within) {
		parts.push(t('profiles.short.within', { value: retention.keep_within }));
	}
	return parts.join(' · ');
}

/** The names of the declared parameters, for the search of a profile (FR-PF-09). */
export function profileParameterNames(profile: ProfileRef): string[] {
	return (profile.parameters ?? []).map((parameter) => parameter.name);
}

/**
 * The secondary line of a profile: the count of its parameters, its path and its retention. A
 * profile has no repository (ADR-0047).
 */
export function profileDescription(profile: ProfileRef): RecordPart[] {
	const chip = parameterChip(profile.parameters ?? []);
	return [
		...(chip ? [{ tag: chip, icon: 'braces' as const }] : []),
		...[profilePathText(profile), profileRetention(profile)].filter(
			(part): part is string => part !== ''
		)
	];
}

export function profileOptions(profiles: readonly ProfileRef[]): RecordOption[] {
	return profiles.map((profile) => {
		const chip = parameterChip(profile.parameters ?? []);
		return {
			value: profile.id,
			label: profile.name,
			description: profileDescription(profile),
			// The closed field shows the first path and the count of the parameters (ADR-0047).
			detail: [
				...(profilePathText(profile) ? [profilePathText(profile)] : []),
				...(chip ? [{ tag: chip, icon: 'braces' as const }] : [])
			],
			// The line shows the first path only, but the search finds each of them and each name.
			search: [...profilePathList(profile), ...profileParameterNames(profile)]
		};
	});
}

/** Gives profile facet values the profile name and its secondary line. */
export function withProfiles(
	options: readonly FilterOption[],
	profiles: readonly ProfileRef[]
): FilterOption[] {
	const byKey = new Map(profiles.map((profile) => [profile.id, profile]));
	return options.map((option) => {
		const profile = byKey.get(option.value);
		return profile
			? { ...option, label: profile.name, description: profileDescription(profile) }
			: option;
	});
}
