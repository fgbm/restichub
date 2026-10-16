import { RuleTester, type Rule } from 'eslint';
import svelteParser from 'svelte-eslint-parser';
import { describe, it } from 'vitest';
import uiPrinciples from '../../eslint-rules/ui-principles.js';

const rules = uiPrinciples.rules as unknown as Record<string, Rule.RuleModule>;

RuleTester.describe = describe;
RuleTester.it = it;
RuleTester.itOnly = it.only;

const tester = new RuleTester({
	languageOptions: { parser: svelteParser, ecmaVersion: 2024, sourceType: 'module' }
});

describe('ui-principles lint rules', () => {
	tester.run('no-raw-table', rules['no-raw-table'], {
		valid: ['<DataTable {columns} {rows} {rowKey} />'],
		invalid: [
			{ code: '<table><tbody></tbody></table>', errors: [{ messageId: 'table' }] },
			{ code: '<Table {columns} />', errors: [{ messageId: 'legacy' }] }
		]
	});

	tester.run('no-row-actions-snippet', rules['no-row-actions-snippet'], {
		valid: ['<DataTable actions={rowActionsOf} />'],
		invalid: [
			{
				code: '<DataTable>{#snippet rowActions(row)}<button>x</button>{/snippet}</DataTable>',
				errors: [{ messageId: 'snippet' }]
			},
			{ code: '<DataTable rowActions={menu} />', errors: [{ messageId: 'snippet' }] }
		]
	});

	tester.run('no-back-link', rules['no-back-link'], {
		valid: [
			"<a href={resolve('/(app)/hosts')}>{t('hp.state.alertsLink')}</a>",
			'<button type="button">{t(\'pagination.previous\')}</button>'
		],
		invalid: [
			{ code: "<a href={href}>{t('hp.back')}</a>", errors: [{ messageId: 'back' }] },
			{ code: "<a href={href}>{t('rp.backToList')}</a>", errors: [{ messageId: 'back' }] },
			{
				code: '<a class="back" href={href}><Icon name="arrow-left" size="xs" /></a>',
				errors: [{ messageId: 'back' }]
			},
			{ code: '<p><a href={href}><span>Назад</span></a></p>', errors: [{ messageId: 'back' }] }
		]
	});
});
