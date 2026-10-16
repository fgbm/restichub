/*
 * A screen reads its data again after each event of the stream. When it assigns the response as
 * is, each item is a new object and each list item renders again, also when nothing changed.
 */

/** True when two API values hold the same data. */
export function sameData(a: unknown, b: unknown): boolean {
	return a === b || JSON.stringify(a) === JSON.stringify(b);
}

/**
 * `next` with each unchanged item replaced by the item of `previous` with the same key, so a keyed
 * list renders only the changed items. Gives `previous` itself when nothing changed. A `live` item
 * is always new: it shows a time that counts from now, such as the duration of a running job.
 */
export function keepUnchanged<T>(
	previous: T[],
	next: T[],
	key: (item: T) => string,
	live: (item: T) => boolean = () => false
): T[] {
	const known = new Map(previous.map((item) => [key(item), item]));
	let same = previous.length === next.length;
	const merged = next.map((item, index) => {
		const old = known.get(key(item));
		const kept = old !== undefined && !live(item) && sameData(old, item) ? old : item;
		if (kept !== previous[index]) same = false;
		return kept;
	});
	return same ? previous : merged;
}
