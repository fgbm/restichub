import { expect, test } from '@playwright/test';
import { mkdirSync } from 'node:fs';

// Captures the tag input in its dialogs for a review. It runs only with CAPTURE and CAPTURE_DIR
// set, so CI skips it (design/style/ui-principles.md, 3.10).
const dir = process.env.CAPTURE_DIR;
test.skip(!process.env.CAPTURE || !dir, 'set CAPTURE and CAPTURE_DIR to write the screenshots');

const out = dir as string;

test('the tag input in Russian, light', async ({ page }) => {
	test.setTimeout(90_000);
	mkdirSync(out, { recursive: true });
	await page.setViewportSize({ width: 1440, height: 900 });
	await page.emulateMedia({ colorScheme: 'light' });
	await page.addInitScript(() => {
		localStorage.setItem('restichub.mock.agents', 'sample');
		localStorage.setItem('restichub.locale', 'ru');
	});
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.locator('form button[type="submit"]').click();
	const nav = page.getByRole('navigation', { name: 'Основная навигация' });
	await expect(nav).toBeVisible();
	await nav.locator('a[href="/hosts"]').first().click();

	await page.getByRole('button', { name: 'Добавить хост' }).click();
	const dialog = page.getByRole('dialog');
	const labels = page.locator('#host-labels');
	await labels.pressSequentially('test, timeweb ');
	await expect(dialog.getByRole('button', { name: 'Убрать timeweb' })).toBeVisible();
	await dialog.screenshot({ path: `${out}/add-host-two-labels.png` });

	await labels.pressSequentially('Prod_1,');
	await expect(dialog.getByRole('alert')).toBeVisible();
	await dialog.screenshot({ path: `${out}/add-host-invalid-label.png` });
	await page.keyboard.press('Escape');

	await nav.getByRole('link', { name: 'Настройки', exact: true }).click();
	await page.getByRole('link', { name: 'Каналы', exact: true }).click();
	await page.getByRole('button', { name: 'Добавить канал' }).first().click();
	const channel = page.getByRole('dialog');
	await page
		.locator('#channel-recipients')
		.pressSequentially('ops@example.com, backup@example.com ');
	await expect(channel.getByRole('button', { name: 'Убрать backup@example.com' })).toBeVisible();
	await channel.screenshot({ path: `${out}/channel-recipients.png` });
});
