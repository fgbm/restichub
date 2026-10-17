/*
 * The open and closed sections of a record editor. The state is kept per kind of editor, not per
 * record, as the list of closed section ids (FR-UI-05).
 */
import {
	isStringArray,
	loadPreference,
	savePreference
} from '#lib/stores/ui-preferences.svelte.ts';

const keyOf = (kind: string) => `editor.${kind}.sections`;

/** Open flags of the sections; an id that the editor no longer has is ignored. */
export function sectionsFrom(
	sections: readonly string[],
	defaults: Record<string, boolean>,
	closed: readonly string[] | null
): Record<string, boolean> {
	if (closed === null) return { ...defaults };
	return Object.fromEntries(sections.map((id) => [id, !closed.includes(id)]));
}

/** Call during the initialization of an editor. Each later change of a flag is kept. */
export function rememberSections(
	kind: string,
	sections: readonly string[],
	defaults: Record<string, boolean>
): Record<string, boolean> {
	const kept = loadPreference<string[] | null>(
		'user',
		keyOf(kind),
		(value): value is string[] | null => value === null || isStringArray(value),
		null
	);
	const open = $state(sectionsFrom(sections, defaults, kept));
	let first = true;
	$effect(() => {
		const closed = sections.filter((id) => !open[id]);
		if (first) {
			first = false;
			return;
		}
		savePreference('user', keyOf(kind), closed);
	});
	return open;
}
