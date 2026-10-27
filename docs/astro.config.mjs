// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import starlightLinksValidator from 'starlight-links-validator';
import { fileURLToPath } from 'node:url';

const page = (slug) => ({ slug });
const subgroup = (en, ru, slugs) => ({
	label: en,
	translations: { ru },
	items: slugs.map(page),
});
const group = (en, ru, items) => ({
	label: en,
	translations: { ru },
	items: items.map((item) => (typeof item === 'string' ? page(item) : item)),
});

// Earlier page addresses stay valid for links outside the site.
const moved = [
	['about', 'getting-started/about'],
	['operations/quick-start', 'getting-started/quick-start'],
	['operations/install-control-plane', 'getting-started/install-control-plane'],
	['operations/register-an-agent', 'getting-started/add-a-host'],
	['operations/migrate-from-resticprofile', 'getting-started/migrate-from-resticprofile'],
	['operations/add-a-repository', 'getting-started/add-a-repository'],
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
	['guides/add-a-repository', 'getting-started/add-a-repository'],
	['guides/enroll-a-host-again', 'guides/agent-certificates'],
	['operations/certificate-rotation', 'guides/agent-certificates'],
	['operations/alert-configuration', 'guides/alerts'],
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
			customCss: ['./src/styles/base.css', './src/styles/theme.css', './src/styles/diagrams.css', './src/styles/screens.css'],
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
					'getting-started/add-a-repository',
					'getting-started/profiles-and-plans',
					'getting-started/migrate-from-resticprofile',
				]),
				group('Concepts', 'Понятия', [
					'concepts/architecture',
					'concepts/trust-model',
					'concepts/repositories',
					'concepts/profiles',
					'concepts/profile-parameters',
					'concepts/plans',
					'concepts/job-lifecycle',
					'concepts/agent-jobs',
				]),
				group('Administration', 'Администрирование', [
					subgroup('Set up', 'Настройка', [
						'guides/organizations',
						'guides/users',
						'guides/api-tokens',
						'guides/alerts',
						'guides/automatic-enrollment',
					]),
					subgroup('Everyday tasks', 'Текущие задачи', [
						'guides/jobs',
						'guides/profile-parameters',
						'guides/restore',
						'guides/scripts',
						'guides/upgrade',
						'guides/agent-certificates',
						'guides/agent-revocation',
					]),
				]),
				group('Operations and security', 'Эксплуатация и безопасность', [
					'operations/security-for-operators',
					'operations/verify-a-release',
					'operations/back-up-the-control-plane',
					'operations/monitor-the-control-plane',
					'operations/root-ca-rotation',
					'operations/job-signing-key-compromise',
					'operations/job-signing-key-recovery',
					'operations/enrollment-rate-limit',
				]),
				group('Reference', 'Справочник', [
					'reference/api',
					'reference/agent-commands',
					'reference/agent-packages',
					'reference/agent-logs',
					'reference/environment-variables',
					'reference/variables',
					'reference/metrics',
					'reference/status-codes',
					'reference/agent-version-compatibility',
					'reference/ui-tour',
					'reference/version-and-support',
					'reference/glossary',
				]),
				{
					label: 'Troubleshooting',
					translations: { ru: 'Устранение неполадок' },
					slug: 'reference/troubleshooting',
				},
			],
		}),
	],
});
