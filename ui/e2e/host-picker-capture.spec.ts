import { expect, test, type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';
import { pickRecord } from './pickers';

// Captures the host picker on the plan page and the host facet of the job list for a review. It
// runs only with CAPTURE and CAPTURE_DIR set, so CI skips it.
const dir = process.env.CAPTURE_DIR;
test.skip(!process.env.CAPTURE || !dir, 'set CAPTURE and CAPTURE_DIR to write the screenshots');

const out = dir as string;

type Labels = { nav: string; plans: string; jobs: string; addPlan: string; host: string };

const EN: Labels = {
	nav: 'Primary navigation',
	plans: 'Plans',
	jobs: 'Jobs',
	addPlan: 'Add plan',
	host: 'Host'
};

const RU: Labels = {
	nav: 'Основная навигация',
	plans: 'Планы',
	jobs: 'Задания',
	addPlan: 'Добавить план',
	host: 'Хост'
};

async function start(page: Page, labels: Labels, locale: 'en' | 'ru'): Promise<void> {
	await page.setViewportSize({ width: 1280, height: 860 });
	await page.addInitScript((locale) => {
		for (const key of ['agents', 'profiles', 'repositories', 'plans', 'backends', 'jobs']) {
			localStorage.setItem(`restichub.mock.${key}`, 'sample');
		}
		localStorage.setItem('restichub.locale', locale);
	}, locale);
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.locator('form button[type="submit"]').click();
	await expect(page.getByRole('navigation', { name: labels.nav })).toBeVisible();
}

async function open(page: Page, labels: Labels, link: string): Promise<void> {
	await page
		.getByRole('navigation', { name: labels.nav })
		.getByRole('link', { name: link, exact: true })
		.click();
	await page.getByRole('heading', { level: 1, name: link }).waitFor();
}

for (const [labels, locale] of [
	[RU, 'ru'],
	[EN, 'en']
] as const) {
	test(`capture the host picker in ${locale}`, async ({ page }) => {
		mkdirSync(out, { recursive: true });
		await start(page, labels, locale);
		await open(page, labels, labels.plans);

		await page.getByRole('button', { name: labels.addPlan, exact: true }).click();
		await page.locator('#plan-agent').click();
		await page.getByRole('listbox').getByRole('option').first().waitFor();
		await page.screenshot({ path: `${out}/1-plan-host-open-${locale}.png` });
		await page.keyboard.press('Escape');

		// A local repository locks the host of the plan (ADR-0046, AC-RP-07).
		await pickRecord(page, page.locator('#plan-repository'), 'nas-local');
		await page.locator('#plan-agent').click();
		await page.getByRole('listbox').getByRole('option').first().waitFor();
		await page.keyboard.type('prod');
		await expect(page.getByRole('listbox').getByRole('option')).toHaveCount(2);
		await page.screenshot({ path: `${out}/2-plan-host-search-disabled-${locale}.png` });
		await page.keyboard.press('Escape');

		page.once('dialog', (dialog) => void dialog.accept());
		await open(page, labels, labels.jobs);
		await page
			.locator('.list-head')
			.getByRole('button', { name: new RegExp(`^${labels.host}`) })
			.click();
		await page.locator('.rh-popover').getByText('win-fs01', { exact: true }).waitFor();
		await page.screenshot({ path: `${out}/3-jobs-host-filter-${locale}.png` });
	});
}
