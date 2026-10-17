import { renderMermaidSVG } from 'beautiful-mermaid';

// The colours stay CSS variables, so the Starlight theme switch recolours the diagram.
// diagrams.css maps them to the --sl-color-* tokens.
const colours = {
	bg: 'var(--bg)',
	fg: 'var(--fg)',
	line: 'var(--line)',
	accent: 'var(--accent)',
	muted: 'var(--muted)',
	surface: 'var(--surface)',
	border: 'var(--border)',
	transparent: true,
	font: 'system-ui',
};

/** Renders Mermaid source to an inline SVG that loads nothing from the network. */
export function renderDiagram(source, label) {
	const svg = renderMermaidSVG(source, colours)
		// The library imports Google Fonts and repeats the colour derivation in each SVG.
		// diagrams.css holds the derivation once.
		.replace(/<style>[\s\S]*?<\/style>/, '')
		.replace(/^<svg ([^>]*?) style="[^"]*"/, '<svg $1');
	if (/@import|url\(['"]?https?:/.test(svg)) {
		throw new Error('the diagram still refers to a remote resource');
	}
	const escaped = label.replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;');
	return svg.replace(/^<svg /, `<svg class="rh-diagram" role="img" aria-label="${escaped}" `);
}
