/*
 * The catalog marks emphasis with <b> and <code>. The parser splits such a text into segments, so a
 * component renders it without raw HTML.
 */
export interface RichSegment {
	text: string;
	tag: 'b' | 'code' | null;
}

export function parseRich(text: string): RichSegment[] {
	const segments: RichSegment[] = [];
	const pattern = /<(b|code)>(.*?)<\/\1>/g;
	let last = 0;
	for (const match of text.matchAll(pattern)) {
		const index = match.index ?? 0;
		if (index > last) segments.push({ text: text.slice(last, index), tag: null });
		segments.push({ text: match[2], tag: match[1] as 'b' | 'code' });
		last = index + match[0].length;
	}
	if (last < text.length) segments.push({ text: text.slice(last), tag: null });
	return segments;
}
