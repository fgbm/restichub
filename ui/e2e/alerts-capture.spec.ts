import { expect, test, type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';

// Captures the dashboard, the alert list and the channel dialog for a review. It runs only with
// CAPTURE and CAPTURE_DIR set, so CI skips it (FR-MN-01, FR-MN-03, FR-MN-04).
const dir = process.env.CAPTURE_DIR;
test.skip(!process.env.CAPTURE || !dir, 'set CAPTURE and CAPTURE_DIR to write the screenshots');
test.describe.configure({ mode: 'serial' });

const out = dir as string;

type Labels = {
	nav: string;
	alerts: string;
	settings: string;
	channels: string;
	addChannel: string;
	openRules: string;
};

const EN: Labels = {
	nav: 'Primary navigation',
	alerts: 'Alerts',
	settings: 'Settings',
	channels: 'Channels',
	addChannel: 'Add channel',
	openRules: 'Alert rules'
};

const RU: Labels = {
	nav: 'Основная навигация',
	alerts: 'Оповещения',
	settings: 'Настройки',
	channels: 'Каналы',
	addChannel: 'Добавить канал',
	openRules: 'Правила оповещений'
};

async function signIn(page: Page, labels: Labels): Promise<void> {
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.locator('form button[type="submit"]').click();
	await expect(page.getByRole('navigation', { name: labels.nav })).toBeVisible();
}

async function shot(page: Page, name: string, scheme: string): Promise<void> {
	await page.waitForTimeout(300);
	await page.screenshot({ path: `${out}/${name}-${scheme}-app.png` });
}

async function capture(page: Page, scheme: string, labels: Labels, suffix: string): Promise<void> {
	mkdirSync(out, { recursive: true });
	await page.setViewportSize({ width: 1440, height: 900 });
	await page.emulateMedia({ colorScheme: scheme });
	await page.addInitScript(() => {
		localStorage.setItem('restichub.mock.agents', 'sample');
		localStorage.setItem('restichub.mock.profiles', 'sample');
		localStorage.setItem('restichub.mock.plans', 'sample');
		localStorage.setItem('restichub.mock.alerts', 'sample');
	});
	if (labels === RU) {
		await page.addInitScript(() => localStorage.setItem('restichub.locale', 'ru'));
	}
	await signIn(page, labels);

	await page.locator('.stats').waitFor();
	await shot(page, `dashboard${suffix}`, scheme);

	await page
		.getByRole('navigation', { name: labels.nav })
		.getByRole('link', { name: labels.alerts, exact: true })
		.click();
	await page.locator('#alerts-status').waitFor();
	await page.getByRole('table').getByText('win-fs01').first().waitFor();
	await shot(page, `alerts-list${suffix}`, scheme);

	await page
		.getByRole('navigation', { name: labels.nav })
		.getByRole('link', { name: labels.settings, exact: true })
		.click();
	await page.getByRole('link', { name: labels.channels, exact: true }).click();
	await page.getByText('ops-hook').waitFor();
	await shot(page, `channels-list${suffix}`, scheme);

	await page.getByRole('button', { name: labels.addChannel, exact: true }).click();
	await page.locator('#channel-name').fill('team-hook');
	await page.locator('#channel-kind').selectOption({ index: 1 });
	await page.locator('#channel-url').fill('https://hooks.example.com/in/token');
	await shot(page, `channel-dialog${suffix}`, scheme);
	await page.keyboard.press('Escape');

	await page.getByRole('link', { name: labels.openRules, exact: true }).click();
	await page.locator('#rule-job_failed').waitFor();
	await shot(page, `rules${suffix}`, scheme);
}

for (const scheme of ['light', 'dark'] as const) {
	test(`capture the alert screens in the ${scheme} theme`, async ({ page }) => {
		await capture(page, scheme, EN, '');
	});
}

test('capture the alert screens in Russian', async ({ page }) => {
	await capture(page, 'light', RU, '-ru');
});
