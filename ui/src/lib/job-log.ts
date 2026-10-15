/*
 * Pure helpers of the job log: which step owns a line, how a filter narrows a log, and when a
 * followed log stops following (FR-LG-01, FR-HK-11).
 */
import type { JobLogLine } from '#lib/api/index.ts';

export type LevelFilter = 'all' | 'problems';

export interface LogGroups {
	/** Lines by step id, in the order of the log. */
	byStep: Map<string, JobLogLine[]>;
	/** Lines of the job itself and lines of a step that the job does not list. */
	general: JobLogLine[];
}

export interface LogCounts {
	total: number;
	warnings: number;
	errors: number;
}

export function groupLines(lines: readonly JobLogLine[], stepIds: readonly string[]): LogGroups {
	const known = new Set(stepIds);
	const byStep = new Map<string, JobLogLine[]>();
	const general: JobLogLine[] = [];
	for (const line of lines) {
		if (!known.has(line.step_id)) {
			general.push(line);
			continue;
		}
		const held = byStep.get(line.step_id);
		if (held) held.push(line);
		else byStep.set(line.step_id, [line]);
	}
	return { byStep, general };
}

export function countLines(lines: readonly JobLogLine[]): LogCounts {
	let warnings = 0;
	let errors = 0;
	for (const line of lines) {
		if (line.level === 'warn') warnings += 1;
		else if (line.level === 'error') errors += 1;
	}
	return { total: lines.length, warnings, errors };
}

export function filterLines(
	lines: readonly JobLogLine[],
	level: LevelFilter,
	query: string
): JobLogLine[] {
	const needle = query.trim().toLowerCase();
	if (level === 'all' && needle === '') return [...lines];
	return lines.filter(
		(line) => (level === 'all' || line.level !== 'info') && line.text.toLowerCase().includes(needle)
	);
}

export function logText(lines: readonly JobLogLine[]): string {
	return lines.map((line) => `${line.ts} ${line.level.toUpperCase()} ${line.text}`).join('\n');
}

// The distance from the end below which a view counts as scrolled to the end.
export const END_MARGIN = 24;

export function scrolledAwayFromEnd(view: {
	scrollHeight: number;
	scrollTop: number;
	clientHeight: number;
}): boolean {
	return view.scrollHeight - view.scrollTop - view.clientHeight > END_MARGIN;
}
