import { expect, test, type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';

// Captures the save bar of the editors and the restic commands panel for a review. It runs only
// with CAPTURE and CAPTURE_DIR set, so CI skips it.
const dir = process.env.CAPTURE_DIR;
test.skip(!process.env.CAPTURE || !dir, 'set CAPTURE and CAPTURE_DIR to write the screenshots');

const out = dir as string;

async function start(page: Page, width: number): Promise<void> {
	mkdirSync(out, { recursive: true });
	page.on('dialog', (dialog) => void dialog.accept().catch(() => undefined));
	await page.setViewportSize({ width, height: 860 });
	await page.addInitScript(() => {
		for (const key of ['agents', 'profiles', 'repositories', 'plans', 'scripts', 'backends']) {
			localStorage.setItem(`restichub.mock.${key}`, 'sample');
		}
		localStorage.setItem('restichub.locale', 'ru');
	});
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.locator('form button[type="submit"]').click();
	await expect(page.getByRole('navigation', { name: 'Основная навигация' })).toBeVisible();
}

async function openProfile(page: Page): Promise<void> {
	await page
		.getByRole('navigation', { name: 'Основная навигация' })
		.getByRole('link', { name: 'Профили', exact: true })
		.click();
	await page.getByRole('link', { name: 'web-daily', exact: true }).click();
	await page.locator('#profile-name').waitFor();
}

test('capture the save bar and the restic commands', async ({ page }) => {
	await start(page, 1280);
	await openProfile(page);
	await page.screenshot({ path: `${out}/1-profile-no-changes-ru.png` });

	await page.getByRole('button', { name: 'Развернуть всё', exact: true }).click();
	await page.locator('#profile-name').fill('web-daily-2');
	await page.mouse.wheel(0, 20_000);
	await page.waitForTimeout(300);
	await page.screenshot({ path: `${out}/2-profile-bottom-changes-ru.png` });
	await page.mouse.wheel(0, -1200);
	await page.waitForTimeout(300);
	await page.screenshot({ path: `${out}/3-profile-scrolling-changes-ru.png` });

	await page.getByRole('button', { name: 'Команды restic', exact: true }).click();
	await page.getByRole('dialog', { name: 'Команды restic' }).waitFor();
	await page.waitForTimeout(300);
	await page.screenshot({ path: `${out}/4-restic-commands-panel-ru.png` });
	await page.keyboard.press('Escape');

	await page
		.getByRole('navigation', { name: 'Основная навигация' })
		.getByRole('link', { name: 'Планы', exact: true })
		.click();
	await page.getByRole('link', { name: 'nightly-web', exact: true }).click();
	await page.locator('#plan-name').fill('nightly-web-2');
	await page.waitForTimeout(300);
	await page.screenshot({ path: `${out}/5-plan-editor-bar-ru.png` });

	await page
		.getByRole('navigation', { name: 'Основная навигация' })
		.getByRole('link', { name: 'Профили', exact: true })
		.click();
	await page.getByRole('button', { name: 'Добавить профиль', exact: true }).click();
	await page.locator('#profile-name').waitFor();
	await page.getByRole('button', { name: 'Команды restic', exact: true }).hover();
	await page.waitForTimeout(800);
	await page.screenshot({ path: `${out}/6-new-profile-commands-disabled-ru.png` });
});

test('capture the save bar at 900 px', async ({ page }) => {
	await start(page, 900);
	await openProfile(page);
	await page.locator('#profile-name').fill('web-daily-2');
	await page.mouse.wheel(0, 20_000);
	await page.waitForTimeout(300);
	await page.screenshot({ path: `${out}/7-profile-900-bottom-ru.png` });
	await page.mouse.wheel(0, -900);
	await page.waitForTimeout(300);
	await page.screenshot({ path: `${out}/8-profile-900-scrolling-ru.png` });
});
