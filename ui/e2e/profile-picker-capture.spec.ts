import { expect, test, type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';

// Captures the profile picker on the plan page and in the profile facet of the plan list for a
// review. It runs only with CAPTURE and CAPTURE_DIR set, so CI skips it.
const dir = process.env.CAPTURE_DIR;
test.skip(!process.env.CAPTURE || !dir, 'set CAPTURE and CAPTURE_DIR to write the screenshots');

const out = dir as string;

type Labels = { nav: string; plans: string; addPlan: string; save: string; find: string };

const EN: Labels = {
	nav: 'Primary navigation',
	plans: 'Plans',
	addPlan: 'Add plan',
	save: 'Save',
	find: 'Find: Profile'
};

const RU: Labels = {
	nav: 'Основная навигация',
	plans: 'Планы',
	addPlan: 'Добавить план',
	save: 'Сохранить',
	find: 'Найти: Профиль'
};

async function start(page: Page, labels: Labels, locale: 'en' | 'ru'): Promise<void> {
	await page.setViewportSize({ width: 1280, height: 860 });
	await page.addInitScript((locale) => {
		for (const key of ['agents', 'profiles', 'repositories', 'plans', 'backends']) {
			localStorage.setItem(`restichub.mock.${key}`, 'sample');
		}
		localStorage.setItem('restichub.locale', locale);
	}, locale);
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.locator('form button[type="submit"]').click();
	await expect(page.getByRole('navigation', { name: labels.nav })).toBeVisible();
	await page
		.getByRole('navigation', { name: labels.nav })
		.getByRole('link', { name: labels.plans, exact: true })
		.click();
	await page.getByRole('heading', { level: 1, name: labels.plans }).waitFor();
}

async function addPlan(page: Page, labels: Labels, name: string, profile: string): Promise<void> {
	await page.getByRole('button', { name: labels.addPlan, exact: true }).click();
	await page.locator('#plan-name').fill(name);
	await page.locator('#plan-profile').click();
	await page.getByRole('option', { name: profile }).click();
	await page.locator('#plan-cron').fill('0 3 * * *');
	await page.getByRole('button', { name: labels.save, exact: true }).click();
	await page.getByRole('link', { name, exact: true }).waitFor();
}

for (const [labels, locale] of [
	[RU, 'ru'],
	[EN, 'en']
] as const) {
	test(`capture the profile picker in ${locale}`, async ({ page }) => {
		mkdirSync(out, { recursive: true });
		await start(page, labels, locale);

		await page.getByRole('button', { name: labels.addPlan, exact: true }).click();
		await page.locator('#plan-profile').click();
		await page.getByRole('listbox').getByRole('option').first().waitFor();
		await page.screenshot({ path: `${out}/1-plan-picker-open-${locale}.png` });

		await page.getByRole('combobox', { name: labels.find }).fill('offsite');
		await expect(page.getByRole('listbox').getByRole('option')).toHaveCount(1);
		await page.screenshot({ path: `${out}/2-plan-picker-search-${locale}.png` });

		await page.keyboard.press('Escape');
		await page.goBack();
		await addPlan(page, labels, 'files-rest', 'win-files');
		await addPlan(page, labels, 'nas-nightly', 'nas-files');
		await page
			.getByRole('button', { name: new RegExp(`^${locale === 'ru' ? 'Профиль' : 'Profile'}`) })
			.click();
		await page.getByText('nas-local').first().waitFor();
		await page.screenshot({ path: `${out}/3-plans-filter-${locale}.png` });
	});
}
