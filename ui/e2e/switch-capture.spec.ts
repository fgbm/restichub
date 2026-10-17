import { expect, test, type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';

// Captures the on/off switch for a review (Russian UI). It runs only with CAPTURE and CAPTURE_DIR
// set, so CI skips it.
const dir = process.env.CAPTURE_DIR;
test.skip(!process.env.CAPTURE || !dir, 'set CAPTURE and CAPTURE_DIR to write the screenshots');
test.describe.configure({ mode: 'serial' });

const out = dir as string;

async function shot(page: Page, name: string): Promise<void> {
	await page.waitForTimeout(300);
	await page.screenshot({ path: `${out}/${name}.png` });
}

test('the switch frames', async ({ page }) => {
	mkdirSync(out, { recursive: true });
	await page.setViewportSize({ width: 1440, height: 900 });
	await page.addInitScript(() => {
		for (const key of ['agents', 'profiles', 'plans', 'repositories', 'alerts']) {
			localStorage.setItem(`restichub.mock.${key}`, 'sample');
		}
		localStorage.setItem('restichub.locale', 'ru');
	});
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.locator('form button[type="submit"]').click();
	await expect(page.getByRole('navigation', { name: 'Основная навигация' })).toBeVisible();

	const nav = page.getByRole('navigation', { name: 'Основная навигация' });
	await nav.getByRole('link', { name: 'Планы', exact: true }).click();
	await expect(page.getByRole('heading', { level: 1, name: 'Планы' })).toBeVisible();
	await page
		.getByRole('list', { name: 'Планы' })
		.getByRole('link', { name: 'nightly-web' })
		.click();
	await page.locator('.runs li').first().waitFor();
	await shot(page, 'plan-editor-on');
	await page.getByRole('switch', { name: 'Запускать по расписанию' }).click();
	await shot(page, 'plan-editor-off');
	await page.getByRole('button', { name: 'Сохранить' }).click();
	await expect(page.getByText('План изменён.')).toBeVisible();
	await expect(page.getByRole('heading', { level: 1, name: 'Планы' })).toBeVisible();
	await page.locator('li.item .status', { hasText: 'Отключён' }).first().waitFor();
	await shot(page, 'plans-list-disabled');

	await nav.getByRole('link', { name: 'Настройки', exact: true }).click();
	await page.getByRole('link', { name: 'Каналы', exact: true }).click();
	await page.locator('li.item').first().getByRole('button').first().click();
	await page.getByRole('dialog').waitFor();
	await shot(page, 'channel-dialog');
});
