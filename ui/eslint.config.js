import js from '@eslint/js';
import prettier from 'eslint-config-prettier';
import svelte from 'eslint-plugin-svelte';
import globals from 'globals';
import tseslint from 'typescript-eslint';
import noHardcodedText from './eslint-rules/no-hardcoded-text.js';
import uiPrinciples from './eslint-rules/ui-principles.js';

export default tseslint.config(
	{
		ignores: ['.svelte-kit/', 'build/', 'node_modules/', 'src/lib/api/schema.d.ts', 'mockups/']
	},
	js.configs.recommended,
	...tseslint.configs.recommended,
	...svelte.configs['flat/recommended'],
	{
		languageOptions: {
			globals: {
				...globals.browser,
				...globals.node
			}
		}
	},
	{
		files: ['**/*.svelte', '**/*.svelte.ts'],
		languageOptions: {
			parserOptions: {
				parser: tseslint.parser
			}
		}
	},
	{
		rules: {
			'@typescript-eslint/no-unused-vars': [
				'error',
				{ argsIgnorePattern: '^_', varsIgnorePattern: '^_' }
			],
			// The production policy is `style-src 'self'` without `unsafe-inline` (ADR-0027).
			// A `style` attribute or a `style:` directive would need `unsafe-inline`.
			'svelte/no-inline-styles': 'error'
		}
	},
	{
		// The templates of the application take their text from the i18n catalogs (AC-UI-03). The
		// component gallery of the dev route shows English sample text on purpose.
		files: ['src/**/*.svelte'],
		ignores: ['src/routes/(dev)/**', 'src/lib/screens/ComponentGallery.svelte'],
		plugins: { local: { rules: { 'no-hardcoded-text': noHardcodedText } } },
		rules: { 'local/no-hardcoded-text': 'error' }
	},
	{
		// design/style/ui-principles.md: tables, row actions and the way back come from the shared
		// components. The sign-in screens have no top bar, so their links back stay.
		files: ['src/**/*.svelte'],
		ignores: ['src/routes/(auth)/**', 'src/lib/components/ui/DataTable.svelte'],
		plugins: { ui: uiPrinciples },
		rules: {
			'ui/no-raw-table': 'error',
			'ui/no-entity-table': 'error',
			'ui/no-row-actions-snippet': 'error',
			'ui/no-back-link': 'error',
			'ui/no-load-more': 'error'
		}
	},
	{
		// design/style/ui-principles.md, 2.15: a file tree is not a list page, so a folder loads its
		// further entries on request.
		files: ['src/lib/components/SnapshotTree.svelte'],
		rules: { 'ui/no-load-more': 'off' }
	},
	prettier
);
