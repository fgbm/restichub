import { expect, test } from '@playwright/test';
import { mkdirSync } from 'node:fs';

// Captures the script templates and the snapshot card for a review. It runs only with CAPTURE and
// CAPTURE_DIR set, so CI skips it.
const dir = process.env.CAPTURE_DIR;
test.skip(!process.env.CAPTURE || !dir, 'set CAPTURE and CAPTURE_DIR to write the screenshots');

const out = dir as string;

test('captures the templates and the snapshot card in Russian, light', async ({ page }) => {
	mkdirSync(out, { recursive: true });
	await page.setViewportSize({ width: 1440, height: 900 });
	await page.emulateMedia({ colorScheme: 'light' });
	await page.addInitScript(() => {
		localStorage.setItem('restichub.mock.agents', 'sample');
		localStorage.setItem('restichub.mock.repositories', 'sample');
		localStorage.setItem('restichub.mock.scripts', 'sample');
		localStorage.setItem('restichub.locale', 'ru');
	});
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.locator('form button[type="submit"]').click();
	await expect(page.getByRole('navigation', { name: 'Основная навигация' })).toBeVisible();

	await page
		.getByRole('navigation', { name: 'Основная навигация' })
		.locator('a[href="/settings"]')
		.first()
		.click();
	await page.locator('a[href="/settings/scripts/templates"]').click();
	await expect(page.locator('.table-scroll tbody tr').first()).toBeVisible();
	await page.screenshot({ path: `${out}/templates-ru-light.png`, fullPage: true });

	await page
		.getByRole('navigation', { name: 'Основная навигация' })
		.locator('a[href="/restore"]')
		.first()
		.click();
	await expect(page.getByRole('link', { name: /^Открыть снимок / }).first()).toBeVisible();
	await page.screenshot({ path: `${out}/snapshots-filled-ru-light.png`, fullPage: true });

	await page.locator('#restore-host').fill('нет-такого-хоста');
	await page.getByRole('button', { name: 'Найти снимки' }).click();
	await expect(page.getByText('Ни один снимок не подходит под фильтры.')).toBeVisible();
	await page.screenshot({ path: `${out}/snapshots-empty-ru-light.png`, fullPage: true });
});
