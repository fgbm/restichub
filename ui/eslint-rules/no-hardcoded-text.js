// eslint-disable-next-line @typescript-eslint/ban-ts-comment
// @ts-nocheck -- an ESLint rule works on untyped syntax tree nodes.
/*
 * AC-UI-03: a template shows no user text that skips the i18n catalogs.
 *
 * The rule reports a text node and a text attribute (`title`, `aria-label`, `placeholder`, `alt`,
 * `label`) of a Svelte template that holds a letter. A text from `t(...)` is an expression, so it
 * passes. A technical value such as a product name needs an `{'...'}` expression or the catalog.
 */
const TEXT_ATTRIBUTES = new Set(['title', 'aria-label', 'placeholder', 'alt', 'label']);
const RAW_TEXT_ELEMENTS = new Set(['script', 'style', 'code', 'pre', 'kbd', 'samp']);

// An example value such as `30d`, `ops@example.com` or `https://host/` is data, not a sentence.
const looksTechnical = (text) => /^[\w.@:/…-]+$/u.test(text) && /[\d@:/.]/.test(text);

const needsCatalog = (text) => /\p{L}/u.test(text) && !looksTechnical(text.trim());

function insideRawElement(node) {
	for (let parent = node.parent; parent; parent = parent.parent) {
		if (parent.type === 'SvelteStyleElement' || parent.type === 'SvelteScriptElement') return true;
		if (parent.type === 'SvelteElement' && RAW_TEXT_ELEMENTS.has(parent.name?.name)) return true;
	}
	return false;
}

/** The string literals that an expression can show as text: a branch of a condition, an operand. */
function shownLiterals(expression, found = []) {
	if (!expression) return found;
	switch (expression.type) {
		case 'Literal':
			if (typeof expression.value === 'string') found.push(expression);
			break;
		case 'TemplateLiteral':
			if (expression.expressions.length === 0) found.push(expression.quasis[0]);
			break;
		case 'ConditionalExpression':
			shownLiterals(expression.consequent, found);
			shownLiterals(expression.alternate, found);
			break;
		case 'LogicalExpression':
			shownLiterals(expression.left, found);
			shownLiterals(expression.right, found);
			break;
		default:
	}
	return found;
}

const literalText = (node) => (node.type === 'TemplateElement' ? node.value.cooked : node.value);

export default {
	meta: {
		type: 'problem',
		schema: [],
		messages: {
			text: 'Move the text "{{text}}" to the i18n catalogs and show it with t().',
			attribute:
				'Move the text of the attribute "{{name}}" to the i18n catalogs and show it with t().'
		}
	},
	create(context) {
		return {
			SvelteText(node) {
				const text = node.value.trim();
				if (!needsCatalog(text) || insideRawElement(node)) return;
				context.report({ node, messageId: 'text', data: { text: text.slice(0, 40) } });
			},
			SvelteMustacheTag(node) {
				if (
					node.kind !== 'text' ||
					node.parent.type === 'SvelteAttribute' ||
					insideRawElement(node)
				)
					return;
				for (const literal of shownLiterals(node.expression)) {
					const text = literalText(literal) ?? '';
					if (needsCatalog(text)) {
						context.report({ node: literal, messageId: 'text', data: { text: text.slice(0, 40) } });
					}
				}
			},
			SvelteAttribute(node) {
				const name = node.key?.name;
				if (typeof name !== 'string' || !TEXT_ATTRIBUTES.has(name)) return;
				for (const value of node.value) {
					if (value.type === 'SvelteLiteral' && needsCatalog(value.value)) {
						context.report({ node: value, messageId: 'attribute', data: { name } });
					}
				}
			}
		};
	}
};
