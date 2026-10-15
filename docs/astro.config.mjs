// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

const group = (en, ru, slugs) => ({
	label: en,
	translations: { ru },
	items: slugs.map((slug) => ({ slug })),
});

// https://starlight.astro.build/reference/configuration/
export default defineConfig({
	site: 'https://fgbm.github.io',
	base: '/restichub',
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
			customCss: ['./src/styles/base.css', './src/styles/theme.css'],
			expressiveCode: { frames: false, styleOverrides: { borderRadius: '6px' } },
			social: [
				{ icon: 'github', label: 'GitHub', href: 'https://github.com/fgbm/restichub' },
			],
			sidebar: [
				group('Getting started', 'Начало работы', [
					'operations/install-control-plane',
					'operations/register-an-agent',
				]),
				group('Daily work', 'Ежедневная работа', [
					'concepts/profiles',
					'concepts/plans',
					'concepts/jobs',
					'operations/agent-jobs',
					'operations/restore',
					'operations/alerts',
					'operations/alert-configuration',
				]),
				group('Hosts and agent', 'Хосты и агент', [
					'operations/agent-packages',
					'operations/agent-version-compatibility',
					'operations/certificate-rotation',
					'operations/agent-revocation',
					'operations/scripts',
				]),
				group('Security', 'Безопасность', [
					'operations/job-signing-key-compromise',
					'operations/job-signing-key-recovery',
					'operations/root-ca-rotation',
					'operations/enrollment-rate-limit',
					'operations/verify-a-release',
				]),
				group('Reference', 'Справочник', [
					'concepts/overview',
					'concepts/repositories',
					'operations/metrics',
				]),
			],
		}),
	],
});
