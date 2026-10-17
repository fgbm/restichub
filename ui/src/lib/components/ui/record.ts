import type { RecordPart } from './types';

export function recordPartText(part: RecordPart): string {
	return typeof part === 'string' ? part : part.tag;
}

/** Case-insensitive match of the label or of any part of the secondary line. */
export function matchesRecord(
	label: string,
	description: readonly RecordPart[] | undefined,
	query: string
): boolean {
	const needle = query.trim().toLowerCase();
	if (needle === '') return true;
	return [label, ...(description ?? []).map(recordPartText)].some((text) =>
		text.toLowerCase().includes(needle)
	);
}
