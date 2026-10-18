import type { RecordPart } from './types';

export function recordPartText(part: RecordPart): string {
	return typeof part === 'string' ? part : part.tag;
}

/** Case-insensitive match of the label, of any part of the secondary line, or of a hidden text. */
export function matchesRecord(
	label: string,
	description: readonly RecordPart[] | undefined,
	query: string,
	extra: readonly string[] = []
): boolean {
	const needle = query.trim().toLowerCase();
	if (needle === '') return true;
	return [label, ...(description ?? []).map(recordPartText), ...extra].some((text) =>
		text.toLowerCase().includes(needle)
	);
}
