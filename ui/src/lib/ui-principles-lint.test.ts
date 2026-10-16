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

	tester.run('no-entity-table', rules['no-entity-table'], {
		valid: [
			'<DataTable {columns} {rows} {rowKey} />',
			'<DataTable {columns} {rows} {rowKey} actions={remove} />',
			'<ListView {items} {itemKey} href={hrefOf} />'
		],
		invalid: [
			{ code: '<DataTable {columns} rowHref={hrefOf} />', errors: [{ messageId: 'entity' }] },
			{ code: '<DataTable {columns} onRowOpen={open} />', errors: [{ messageId: 'entity' }] },
			{ code: '<DataTable {columns} {detailTitle} />', errors: [{ messageId: 'entity' }] }
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

	tester.run('no-load-more', rules['no-load-more'], {
		valid: [
			"<Button onclick={next}>{t('pagination.next')}</Button>",
			"<p>{t('dashboard.more')}</p>"
		],
		invalid: [
			{
				code: "<Button onclick={loadMore}>{t('common.loadMore')}</Button>",
				errors: [{ messageId: 'more' }]
			},
			{ code: "<button>{t('jobs.showMore')}</button>", errors: [{ messageId: 'more' }] },
			{ code: '<button>Загрузить ещё</button>', errors: [{ messageId: 'more' }] }
		]
	});
});
