import { expect, test } from '@playwright/test';
import { mkdirSync } from 'node:fs';

// Captures the password rule of the create dialog for a review. It runs only with CAPTURE and
// CAPTURE_DIR set, so CI skips it.
const dir = process.env.CAPTURE_DIR;
test.skip(!process.env.CAPTURE || !dir, 'set CAPTURE and CAPTURE_DIR to write the screenshots');

const out = dir as string;

test('captures the password rule of the platform create dialog', async ({ page }) => {
	mkdirSync(out, { recursive: true });
	await page.setViewportSize({ width: 1440, height: 900 });
	await page.emulateMedia({ colorScheme: 'light' });
	await page.addInitScript(() => {
		localStorage.setItem('restichub.theme', 'light');
		localStorage.setItem('restichub.locale', 'ru');
	});
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.locator('form button[type="submit"]').click();
	await expect(page.getByRole('navigation', { name: 'Основная навигация' })).toBeVisible();

	await page.getByRole('button', { name: /Сменить организацию/ }).click();
	await page.getByRole('menuitem', { name: 'Администрирование платформы' }).click();
	const nav = page.getByRole('navigation', { name: 'Основная навигация' });
	await nav.getByRole('link', { name: 'Пользователи', exact: true }).click();
	await page.getByRole('heading', { level: 1, name: 'Пользователи' }).waitFor();

	await page.getByRole('button', { name: 'Создать пользователя' }).click();
	const dialog = page.getByRole('dialog');
	await dialog.waitFor();
	const shot = async (name: string) => {
		await page.waitForTimeout(300);
		await dialog.screenshot({ path: `${out}/${name}.png` });
	};
	await shot('1-empty');

	await page.locator('#platform-user-username').fill('ivan');
	await page.locator('#platform-user-display-name').fill('Иван Егоркин');
	await page.locator('#platform-user-password').pressSequentially('abcdef');
	await shot('2-six-characters');

	await dialog.getByRole('button', { name: 'Создать пользователя' }).click();
	await shot('3-six-characters-after-submit');

	await page.locator('#platform-user-password').pressSequentially('ghijklmn');
	await shot('4-fourteen-characters');
});
