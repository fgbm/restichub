import { expect, test, type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';

// Captures the host page and the hosts list in Russian for a review. It runs only with CAPTURE
// and CAPTURE_DIR set, so CI skips it (FR-ID-11).
const dir = process.env.CAPTURE_DIR;
test.skip(!process.env.CAPTURE || !dir, 'set CAPTURE and CAPTURE_DIR to write the screenshots');
test.describe.configure({ mode: 'serial' });

const out = dir as string;
const width = Number(process.env.CAPTURE_WIDTH ?? 1440);

async function shot(page: Page, name: string, full = false): Promise<void> {
	await page.waitForTimeout(250);
	await page.screenshot({ path: `${out}/${name}-${width}.png`, fullPage: full });
}

async function openHosts(page: Page): Promise<void> {
	await page
		.getByRole('navigation', { name: 'Основная навигация' })
		.getByRole('link', { name: 'Хосты', exact: true })
		.click();
	await page.getByRole('link', { name: 'win-fs01', exact: true }).waitFor();
}

test('capture the host page and the hosts list', async ({ page }) => {
	mkdirSync(out, { recursive: true });
	await page.setViewportSize({ width, height: 900 });
	await page.emulateMedia({ colorScheme: 'light' });
	await page.addInitScript(() => {
		localStorage.setItem('restichub.mock.agents', 'sample');
		localStorage.setItem('restichub.mock.profiles', 'sample');
		localStorage.setItem('restichub.mock.plans', 'sample');
		localStorage.setItem('restichub.mock.alerts', 'sample');
		localStorage.setItem('restichub.mock.scripts', 'sample');
		localStorage.setItem('restichub.mock.repositories', 'sample');
		localStorage.setItem('restichub.locale', 'ru');
	});
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.locator('form button[type="submit"]').click();
	await expect(page.getByRole('navigation', { name: 'Основная навигация' })).toBeVisible();

	await openHosts(page);
	await shot(page, 'hosts-list');
	await page.getByRole('button', { name: 'Другие действия: win-fs01', exact: true }).click();
	await shot(page, 'hosts-list-menu');
	await page.keyboard.press('Escape');

	await page.getByRole('link', { name: 'srv-db-01', exact: true }).click();
	await expect(page.getByRole('heading', { level: 1, name: 'srv-db-01' })).toBeVisible();
	await page.waitForTimeout(300);
	await shot(page, 'host-online', true);
	await page.locator('#scripts').scrollIntoViewIfNeeded();
	await shot(page, 'host-trust-mode');

	await page.getByRole('link', { name: 'К списку хостов' }).click();
	await page.getByRole('link', { name: 'win-fs01-new', exact: true }).click();
	await expect(page.getByRole('heading', { level: 1, name: 'win-fs01-new' })).toBeVisible();
	await page.waitForTimeout(300);
	await shot(page, 'host-offline-duplicate', true);
});
