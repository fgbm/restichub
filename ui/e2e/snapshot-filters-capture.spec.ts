import { expect, test, type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';

// Captures the filter bar of the snapshot list for a review (FR-RS-01). It runs only with CAPTURE
// and CAPTURE_DIR set, so CI skips it.
const dir = process.env.CAPTURE_DIR;
test.skip(!process.env.CAPTURE || !dir, 'set CAPTURE and CAPTURE_DIR to write the screenshots');

const out = dir as string;

async function capture(page: Page, width: number): Promise<void> {
	mkdirSync(out, { recursive: true });
	const size = width < 600 ? 'mobile' : 'desktop';
	await page.setViewportSize({ width, height: width < 600 ? 800 : 900 });
	await page.emulateMedia({ colorScheme: 'light' });
	await page.addInitScript(() => {
		localStorage.setItem('restichub.mock.agents', 'sample');
		localStorage.setItem('restichub.mock.repositories', 'sample');
		localStorage.setItem('restichub.mock.snapshots', 'many');
		localStorage.setItem('restichub.theme', 'light');
		localStorage.setItem('restichub.locale', 'ru');
	});
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.locator('form button[type="submit"]').click();
	await page
		.getByRole('navigation', { name: 'Основная навигация' })
		.getByRole('link', { name: 'Восстановление', exact: true })
		.click();

	const rows = page.getByRole('link', { name: /^Открыть снимок / });
	await expect(rows.first()).toBeVisible();
	await page.waitForTimeout(300);
	await page.screenshot({ path: `${out}/1-default-${size}.png`, fullPage: true });

	await page.getByRole('button', { name: 'Хост', exact: true }).click();
	await expect(page.getByLabel('MindCore')).toBeVisible();
	await page.screenshot({ path: `${out}/3-host-popover-${size}.png` });
	await page.getByLabel('MindCore').check();
	await page.keyboard.press('Escape');

	await page.getByRole('button', { name: 'Период', exact: true }).click();
	await page.screenshot({ path: `${out}/4-period-popover-${size}.png` });
	await page.getByRole('button', { name: 'Последние 30 дней' }).click();
	await expect(page.getByText('Хост: MindCore')).toBeVisible();
	await page.waitForTimeout(300);
	await page.screenshot({ path: `${out}/2-two-filters-${size}.png`, fullPage: true });

	await page.getByRole('button', { name: 'Период', exact: true }).click();
	await page.getByRole('button', { name: 'Свой диапазон…' }).click();
	await page.screenshot({ path: `${out}/4b-period-custom-${size}.png` });
	await page.keyboard.press('Escape');

	await page.locator('#restore-path').fill('нет-такого-пути');
	await expect(page.getByText('Ни один снимок не подходит под фильтры.')).toBeVisible();
	await page.screenshot({ path: `${out}/5-empty-${size}.png`, fullPage: true });

	const overflow = await page.evaluate(
		() => document.documentElement.scrollWidth - document.documentElement.clientWidth
	);
	expect(overflow).toBeLessThanOrEqual(0);
}

test('the snapshot filter bar at 1440', async ({ page }) => {
	await capture(page, 1440);
});

test('the snapshot filter bar at 390', async ({ page }) => {
	await capture(page, 390);
});
