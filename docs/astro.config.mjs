// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

// https://starlight.astro.build/reference/configuration/
export default defineConfig({
	site: 'https://fgbm.github.io',
	base: '/restichub',
	integrations: [
		starlight({
			title: 'ResticHub',
			defaultLocale: 'root',
			locales: {
				root: { label: 'English', lang: 'en' },
				ru: { label: 'Русский', lang: 'ru' },
			},
			social: [
				{ icon: 'github', label: 'GitHub', href: 'https://github.com/fgbm/restichub' },
			],
			sidebar: [
				{
					label: 'Concepts',
					translations: { ru: 'Основные понятия' },
					items: [{ autogenerate: { directory: 'concepts' } }],
				},
				{
					label: 'Operations',
					translations: { ru: 'Операции' },
					items: [{ autogenerate: { directory: 'operations' } }],
				},
			],
		}),
	],
});
