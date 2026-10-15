// Reads the symbols of the mockup sprite. The symbol id `i-host` becomes the icon name `host`.
/**
 * @param {string} source
 * @returns {Record<string, [string, Record<string, string>][]>}
 */
export function parseSprite(source) {
	/** @type {Record<string, [string, Record<string, string>][]>} */
	const icons = {};
	for (const symbol of source.matchAll(/<symbol id="i-([a-z0-9-]+)"[^>]*>([\s\S]*?)<\/symbol>/g)) {
		/** @type {[string, Record<string, string>][]} */
		const shapes = [];
		for (const shape of symbol[2].matchAll(
			/<(path|circle|rect|ellipse|line|polyline|polygon)\b([^>]*?)\/>/g
		)) {
			/** @type {Record<string, string>} */
			const attributes = {};
			for (const attribute of shape[2].matchAll(/([a-z0-9-]+)="([^"]*)"/g)) {
				attributes[attribute[1]] = attribute[2];
			}
			shapes.push([shape[1], attributes]);
		}
		icons[symbol[1]] = shapes;
	}
	return icons;
}
