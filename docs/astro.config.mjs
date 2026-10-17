// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import starlightLinksValidator from 'starlight-links-validator';
import { fileURLToPath } from 'node:url';

const group = (en, ru, slugs) => ({
	label: en,
	translations: { ru },
	items: slugs.map((slug) => ({ slug })),
});

// Earlier page addresses stay valid for links outside the site.
const moved = [
	['about', 'getting-started/about'],
	['operations/quick-start', 'getting-started/quick-start'],
	['operations/install-control-plane', 'getting-started/install-control-plane'],
	['operations/register-an-agent', 'getting-started/add-a-host'],
	['operations/migrate-from-resticprofile', 'getting-started/migrate-from-resticprofile'],
	['operations/add-a-repository', 'guides/add-a-repository'],
	['operations/restore', 'guides/restore'],
	['operations/scripts', 'guides/scripts'],
	['operations/alerts', 'guides/alerts'],
	['operations/users', 'guides/users'],
	['operations/enroll-a-host-again', 'guides/enroll-a-host-again'],
	['operations/agent-revocation', 'guides/agent-revocation'],
	['concepts/overview', 'concepts/architecture'],
	['concepts/jobs', 'concepts/job-lifecycle'],
	['operations/agent-jobs', 'concepts/agent-jobs'],
	['operations/agent-packages', 'reference/agent-packages'],
	['operations/environment-variables', 'reference/environment-variables'],
	['operations/metrics', 'reference/metrics'],
	['operations/api-reference', 'reference/api'],
	['operations/agent-version-compatibility', 'reference/agent-version-compatibility'],
	['operations/glossary', 'reference/glossary'],
];

const redirects = Object.fromEntries(
	moved.flatMap(([from, to]) => [
		[`/${from}`, `/restichub/${to}/`],
		[`/ru/${from}`, `/restichub/ru/${to}/`],
	]),
);

// https://starlight.astro.build/reference/configuration/
export default defineConfig({
	site: 'https://fgbm.github.io',
	base: '/restichub',
	redirects,
	vite: {
		define: {
			'import.meta.env.UI_I18N_DIR': JSON.stringify(fileURLToPath(new URL('../ui/src/lib/i18n/', import.meta.url))),
		},
	},
	integrations: [
		starlight({
			title: 'ResticHub',
			logo: { src: './src/assets/logo.svg' },
			favicon: '/favicon.svg',
			defaultLocale: 'root',
			locales: {
				root: { label: 'English', lang: 'en' },
				ru: { label: 'Русский', lang: 'ru' },
			},
			plugins: [starlightLinksValidator()],
			customCss: ['./src/styles/base.css', './src/styles/theme.css', './src/styles/diagrams.css'],
			expressiveCode: { frames: false, styleOverrides: { borderRadius: '6px' } },
			social: [
				{ icon: 'github', label: 'GitHub', href: 'https://github.com/fgbm/restichub' },
			],
			sidebar: [
				group('Getting started', 'Начало работы', [
					'getting-started/about',
					'getting-started/quick-start',
					'getting-started/install-control-plane',
					'getting-started/add-a-host',
					'getting-started/migrate-from-resticprofile',
				]),
				group('Guides', 'Руководства', [
					'guides/add-a-repository',
					'guides/jobs',
					'guides/restore',
					'guides/scripts',
					'guides/alerts',
					'guides/users',
					'guides/organizations',
					'guides/api-tokens',
					'guides/upgrade',
					'guides/enroll-a-host-again',
					'guides/agent-revocation',
				]),
				group('Concepts', 'Понятия', [
					'concepts/architecture',
					'concepts/job-lifecycle',
					'concepts/agent-jobs',
					'concepts/profiles',
					'concepts/plans',
					'concepts/repositories',
					'concepts/trust-model',
				]),
				group('Operations and security', 'Эксплуатация и безопасность', [
					'operations/back-up-the-control-plane',
					'operations/certificate-rotation',
					'operations/root-ca-rotation',
					'operations/job-signing-key-compromise',
					'operations/job-signing-key-recovery',
					'operations/enrollment-rate-limit',
					'operations/verify-a-release',
					'operations/security-for-operators',
					'operations/alert-configuration',
				]),
				group('Reference', 'Справочник', [
					'reference/agent-commands',
					'reference/agent-packages',
					'reference/agent-logs',
					'reference/environment-variables',
					'reference/metrics',
					'reference/api',
					'reference/status-codes',
					'reference/agent-version-compatibility',
					'reference/troubleshooting',
					'reference/glossary',
				]),
			],
		}),
	],
});
