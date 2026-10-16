import js from '@eslint/js';
import prettier from 'eslint-config-prettier';
import svelte from 'eslint-plugin-svelte';
import globals from 'globals';
import tseslint from 'typescript-eslint';
import noHardcodedText from './eslint-rules/no-hardcoded-text.js';
import uiPrinciples from './eslint-rules/ui-principles.js';

// The screens that still use the older Table and their own back links; the migration to
// DataTable, Page and RecordPage empties this list.
const legacyScreens = [
	'src/lib/components/ui/Table.svelte',
	'src/lib/components/HostPage.svelte',
	'src/lib/components/ProfileEditor.svelte',
	'src/lib/components/RepositoryEditor.svelte',
	'src/lib/screens/AuditLog.svelte',
	'src/routes/(app)/+page.svelte',
	'src/routes/(app)/alerts/+page.svelte',
	'src/routes/(app)/hosts/+page.svelte',
	'src/routes/(app)/jobs/+page.svelte',
	'src/routes/(app)/plans/+page.svelte',
	'src/routes/(app)/platform/tenants/+page.svelte',
	'src/routes/(app)/platform/users/+page.svelte',
	'src/routes/(app)/profiles/+page.svelte',
	'src/routes/(app)/repositories/+page.svelte',
	'src/routes/(app)/restore/[[]repository]/[[]snapshot]/+page.svelte',
	'src/routes/(app)/settings/access/api-tokens/+page.svelte',
	'src/routes/(app)/settings/access/secrets/+page.svelte',
	'src/routes/(app)/settings/access/users/+page.svelte',
	'src/routes/(app)/settings/notifications/channels/+page.svelte',
	'src/routes/(app)/settings/scripts/jobs/+page.svelte',
	'src/routes/(app)/settings/scripts/templates/+page.svelte'
];

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
		ignores: [
			'src/routes/(dev)/**',
			'src/routes/(auth)/**',
			'src/lib/screens/ComponentGallery.svelte',
			'src/lib/components/ui/DataTable.svelte',
			...legacyScreens
		],
		plugins: { ui: uiPrinciples },
		rules: {
			'ui/no-raw-table': 'error',
			'ui/no-row-actions-snippet': 'error',
			'ui/no-back-link': 'error'
		}
	},
	prettier
);
