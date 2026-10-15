import { expect, test } from '@playwright/test';
import { mkdirSync } from 'node:fs';

// Captures the host screens for a review. It runs only with CAPTURE and CAPTURE_DIR set, so CI
// skips it (FR-ID-01).
const dir = process.env.CAPTURE_DIR;
test.skip(!process.env.CAPTURE || !dir, 'set CAPTURE and CAPTURE_DIR to write the screenshots');

const out = dir as string;

test('the host screens in Russian, light', async ({ page }) => {
	test.setTimeout(90_000);
	mkdirSync(out, { recursive: true });
	await page.setViewportSize({ width: 1440, height: 900 });
	await page.emulateMedia({ colorScheme: 'light' });
	await page.addInitScript(() => {
		localStorage.setItem('restichub.mock.agents', 'sample');
		localStorage.setItem('restichub.mock.enroll', 'soon');
		localStorage.setItem('restichub.locale', 'ru');
	});
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.locator('form button[type="submit"]').click();
	const nav = page.getByRole('navigation', { name: 'Основная навигация' });
	await expect(nav).toBeVisible();
	await nav.locator('a[href="/hosts"]').first().click();
	await expect(page.getByText('ws-reception').first()).toBeVisible();
	await page.screenshot({ path: `${out}/hosts-possible-duplicate.png` });
	await page.getByText('ws-reception').first().scrollIntoViewIfNeeded();
	await page.screenshot({ path: `${out}/hosts-waiting.png` });

	await page.getByRole('button', { name: 'Добавить хост' }).click();
	await page.getByRole('button', { name: /Создать токен/ }).click();
	await expect(page.getByText('Хост зарегистрирован: ws-reception')).toBeVisible({
		timeout: 40_000
	});
	await page.screenshot({ path: `${out}/add-host-registered.png` });
});
