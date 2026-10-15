import { expect, test } from '@playwright/test';
import { mkdirSync } from 'node:fs';

// Captures the Secrets screen as screenshots. It runs only with CAPTURE and CAPTURE_DIR set, so CI
// skips it.
const dir = process.env.CAPTURE_DIR;
test.skip(!process.env.CAPTURE || !dir, 'set CAPTURE and CAPTURE_DIR to write the screenshots');

const out = dir as string;

test('captures the Secrets screen in Russian', async ({ page }) => {
	mkdirSync(out, { recursive: true });
	await page.setViewportSize({ width: 1440, height: 900 });
	await page.emulateMedia({ colorScheme: 'light' });
	await page.addInitScript(() => {
		localStorage.setItem('restichub.mock.agents', 'sample');
		localStorage.setItem('restichub.mock.profiles', 'sample');
		localStorage.setItem('restichub.mock.scripts', 'sample');
		localStorage.setItem('restichub.mock.repositories', 'sample');
		localStorage.setItem('restichub.locale', 'ru');
	});
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.locator('form button[type="submit"]').click();
	await expect(page.getByRole('navigation', { name: 'Основная навигация' })).toBeVisible();
	await page.locator('a[href="/settings"]').first().click();
	await page.locator('a[href="/settings/access/secrets"]').click();
	await page.locator('table').waitFor();
	await page.waitForTimeout(300);
	await page.screenshot({ path: `${out}/secrets-ru-light.png` });

	await page.getByRole('button', { name: 'Добавить секрет' }).first().click();
	await page.waitForTimeout(300);
	await page.screenshot({ path: `${out}/secrets-create-ru-light.png` });
	await page.keyboard.press('Escape');

	const row = page.getByRole('row').filter({ hasText: 'spare.token' });
	await row.getByRole('button', { name: 'Заменить значение' }).click();
	await page.waitForTimeout(300);
	await page.screenshot({ path: `${out}/secrets-replace-ru-light.png` });
	await page.keyboard.press('Escape');

	await page
		.getByRole('row')
		.filter({ hasText: 'db.password' })
		.getByRole('button', { name: 'Удалить' })
		.click();
	await page.waitForTimeout(300);
	await page.screenshot({ path: `${out}/secrets-delete-in-use-ru-light.png` });
});
