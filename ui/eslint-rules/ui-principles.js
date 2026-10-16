// eslint-disable-next-line @typescript-eslint/ban-ts-comment
// @ts-nocheck -- an ESLint rule works on untyped syntax tree nodes.
/*
 * Guards for design/style/ui-principles.md. A page gets the table, the row actions and the way back
 * from the shared components, so these rules report markup that builds them by hand.
 */

const elementName = (node) => (node.name?.type === 'Identifier' ? node.name.name : node.name?.name);

/** UI principles 2: a table is a DataTable, which applies the row and the action rules. */
const noRawTable = {
	meta: {
		type: 'problem',
		schema: [],
		messages: {
			table: 'Use DataTable instead of a raw <table> (design/style/ui-principles.md, section 2).',
			legacy: 'Use DataTable instead of Table (design/style/ui-principles.md, section 2).'
		}
	},
	create(context) {
		return {
			SvelteElement(node) {
				const name = elementName(node);
				if (node.kind === 'html' && name === 'table') {
					context.report({ node, messageId: 'table' });
				} else if (node.kind === 'component' && name === 'Table') {
					context.report({ node, messageId: 'legacy' });
				}
			}
		};
	}
};

/** UI principles 2.3: row actions are declared with the `actions` property of DataTable. */
const noRowActionsSnippet = {
	meta: {
		type: 'problem',
		schema: [],
		messages: {
			snippet:
				'Declare the row actions with the DataTable property `actions` (design/style/ui-principles.md, 2.3).'
		}
	},
	create(context) {
		return {
			SvelteSnippetBlock(node) {
				if (node.id?.name === 'rowActions') context.report({ node, messageId: 'snippet' });
			},
			SvelteAttribute(node) {
				if (node.key?.name === 'rowActions') context.report({ node, messageId: 'snippet' });
			}
		};
	}
};

// A message key such as `hp.back`, `rp.backToList` or `jobs.toList`.
const BACK_KEY = /(^|\.)(back|toList)([A-Z][\w]*)?$/;
const BACK_TEXT = /^\s*(←|back\b|назад|к списку)/iu;

function isBackText(child) {
	if (child.type === 'SvelteText') return BACK_TEXT.test(child.value);
	if (child.type === 'SvelteMustacheTag') {
		const call = child.expression;
		if (call?.type !== 'CallExpression' || call.callee?.name !== 't') return false;
		const key = call.arguments[0];
		return key?.type === 'Literal' && typeof key.value === 'string' && BACK_KEY.test(key.value);
	}
	if (child.type === 'SvelteElement' && elementName(child) === 'Icon') {
		return child.startTag.attributes.some(
			(attribute) =>
				attribute.key?.name === 'name' &&
				attribute.value?.some(
					(part) => part.type === 'SvelteLiteral' && part.value === 'arrow-left'
				)
		);
	}
	return false;
}

function holdsBackText(node) {
	return node.children.some(
		(child) => isBackText(child) || (child.type === 'SvelteElement' && holdsBackText(child))
	);
}

/** UI principles 1.1: the breadcrumb in the top bar is the only way back. */
const noBackLink = {
	meta: {
		type: 'problem',
		schema: [],
		messages: {
			back: 'Remove the back link: the breadcrumb is the only way back (design/style/ui-principles.md, 1.1). Declare the levels with Page or RecordPage.'
		}
	},
	create(context) {
		return {
			SvelteElement(node) {
				if (node.kind !== 'html' || elementName(node) !== 'a') return;
				if (holdsBackText(node)) context.report({ node, messageId: 'back' });
			}
		};
	}
};

export default {
	rules: {
		'no-raw-table': noRawTable,
		'no-row-actions-snippet': noRowActionsSnippet,
		'no-back-link': noBackLink
	}
};
