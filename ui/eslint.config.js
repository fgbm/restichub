import js from '@eslint/js';
import prettier from 'eslint-config-prettier';
import svelte from 'eslint-plugin-svelte';
import globals from 'globals';
import tseslint from 'typescript-eslint';
import noHardcodedText from './eslint-rules/no-hardcoded-text.js';

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
	prettier
);
