import { expect, test, type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';

// Captures the script screens for a review. It runs only with CAPTURE and CAPTURE_DIR set, so CI
// skips it (FR-HK-09, FR-HK-11, FR-HK-12).
const dir = process.env.CAPTURE_DIR;
test.skip(!process.env.CAPTURE || !dir, 'set CAPTURE and CAPTURE_DIR to write the screenshots');
test.describe.configure({ mode: 'serial' });

const out = dir as string;

type Labels = { nav: string; openJob: RegExp; edit: string; actions: string };

const EN: Labels = {
	nav: 'Primary navigation',
	openJob: /Open the job nightly-db/,
	edit: 'Edit',
	actions: 'More actions for db-nightly'
};

const RU: Labels = {
	nav: 'Основная навигация',
	openJob: /Открыть задание nightly-db/,
	edit: 'Изменить',
	actions: 'Другие действия: db-nightly'
};

async function start(page: Page, scheme: string, labels: Labels): Promise<void> {
	mkdirSync(out, { recursive: true });
	await page.setViewportSize({ width: 1440, height: 900 });
	await page.emulateMedia({ colorScheme: scheme as 'light' | 'dark' });
	await page.addInitScript(() => {
		localStorage.setItem('restichub.mock.agents', 'sample');
		localStorage.setItem('restichub.mock.profiles', 'sample');
		localStorage.setItem('restichub.mock.plans', 'sample');
		localStorage.setItem('restichub.mock.scripts', 'sample');
	});
	if (labels === RU) {
		await page.addInitScript(() => localStorage.setItem('restichub.locale', 'ru'));
	}
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.locator('form button[type="submit"]').click();
	await expect(page.getByRole('navigation', { name: labels.nav })).toBeVisible();
}

async function go(page: Page, labels: Labels, path: string): Promise<void> {
	await page
		.getByRole('navigation', { name: labels.nav })
		.locator(`a[href="${path}"]`)
		.first()
		.click();
}

async function captureProfile(page: Page, scheme: string, labels: Labels, suffix: string) {
	await start(page, scheme, labels);
	await go(page, labels, '/profiles');
	await page.getByRole('button', { name: labels.actions }).click();
	await page.getByRole('menuitem', { name: labels.edit }).click();
	await page.getByRole('button', { name: /^(Scripts on the host|Скрипты на хосте)/ }).click();
	const editor = page.getByTestId('scripts-editor');
	await editor.waitFor();
	// The second script shows its editor, so the capture holds the line numbers and the parameters.
	await page.getByTestId('script-card').nth(1).locator('.card__head button').first().click();
	// The section starts at the top of the page.
	await page
		.locator('[data-section="scripts"]')
		.evaluate((element) => element.scrollIntoView({ block: 'start' }));
	await page.waitForTimeout(300);
	await page.screenshot({ path: `${out}/profile-scripts-${scheme}${suffix}.png` });
}

async function captureTemplates(page: Page, scheme: string, labels: Labels, suffix: string) {
	await start(page, scheme, labels);
	await go(page, labels, '/settings');
	await page.locator('a[href="/settings/scripts/templates"]').click();
	await page.locator('table').waitFor();
	await page.waitForTimeout(300);
	await page.screenshot({ path: `${out}/script-templates-${scheme}${suffix}.png` });
}

async function captureJob(page: Page, scheme: string, labels: Labels, suffix: string) {
	await start(page, scheme, labels);
	await go(page, labels, '/jobs');
	await page.getByRole('link', { name: labels.openJob }).first().click();
	await page.locator('.timeline__toggle').first().waitFor();
	for (const index of [1, 2]) {
		await page.locator('.timeline__toggle').nth(index).click();
	}
	await page.locator('.step-log__line').first().waitFor();
	await page.waitForTimeout(300);
	await page.screenshot({ path: `${out}/job-steps-${scheme}${suffix}.png` });
}

test('captures the profile scripts in English, light', async ({ page }) => {
	await captureProfile(page, 'light', EN, '-en');
});

test('captures the profile scripts in Russian, light', async ({ page }) => {
	await captureProfile(page, 'light', RU, '-ru');
});

test('captures the script templates in English, light', async ({ page }) => {
	await captureTemplates(page, 'light', EN, '-en');
});

test('captures the script templates in Russian, light', async ({ page }) => {
	await captureTemplates(page, 'light', RU, '-ru');
});

test('captures the job steps in English, light', async ({ page }) => {
	await captureJob(page, 'light', EN, '-en');
});

test('captures the job steps in Russian, light', async ({ page }) => {
	await captureJob(page, 'light', RU, '-ru');
});
