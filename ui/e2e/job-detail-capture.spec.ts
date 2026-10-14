import { expect, test, type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';

// Captures the job detail screen for a review. It runs only with CAPTURE and CAPTURE_DIR set, so
// CI skips it (FR-MN-02, FR-LG-01).
const dir = process.env.CAPTURE_DIR;
test.skip(!process.env.CAPTURE || !dir, 'set CAPTURE and CAPTURE_DIR to write the screenshots');
test.describe.configure({ mode: 'serial' });

const out = dir as string;

type Labels = {
	nav: string;
	jobs: string;
	open: RegExp;
	running: string;
	succeeded: string;
};

const EN: Labels = {
	nav: 'Primary navigation',
	jobs: 'Jobs',
	open: /Open the job nightly-web/,
	running: 'Running',
	succeeded: 'Succeeded'
};

const RU: Labels = {
	nav: 'Основная навигация',
	jobs: 'Задания',
	open: /Открыть задание nightly-web/,
	running: 'Выполняется',
	succeeded: 'Успешно'
};

async function signIn(page: Page, labels: Labels): Promise<void> {
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.locator('form button[type="submit"]').click();
	await expect(page.getByRole('navigation', { name: labels.nav })).toBeVisible();
}

async function capture(page: Page, scheme: string, labels: Labels, suffix: string): Promise<void> {
	mkdirSync(out, { recursive: true });
	await page.setViewportSize({ width: 1440, height: 900 });
	await page.emulateMedia({ colorScheme: scheme });
	await page.addInitScript(() => {
		localStorage.setItem('restichub.mock.agents', 'sample');
		localStorage.setItem('restichub.mock.profiles', 'sample');
		localStorage.setItem('restichub.mock.plans', 'sample');
	});
	if (labels === RU) {
		await page.addInitScript(() => localStorage.setItem('restichub.locale', 'ru'));
	}
	await signIn(page, labels);
	await page
		.getByRole('navigation', { name: labels.nav })
		.getByRole('link', { name: labels.jobs, exact: true })
		.click();

	for (const [status, name] of [
		[labels.running, 'running'],
		[labels.succeeded, 'finished']
	] as const) {
		await page.locator('#jobs-status').selectOption({ label: status });
		await page.getByRole('link', { name: labels.open }).first().click();
		await page.locator('.log__line').first().waitFor();
		await page.waitForTimeout(300);
		await page.screenshot({ path: `${out}/job-${name}-${scheme}${suffix}.png` });
		await page.goBack();
	}
}

test('captures the job detail in English, light', async ({ page }) => {
	await capture(page, 'light', EN, '-en');
});

test('captures the job detail in English, dark', async ({ page }) => {
	await capture(page, 'dark', EN, '-en');
});

test('captures the job detail in Russian, light', async ({ page }) => {
	await capture(page, 'light', RU, '-ru');
});
