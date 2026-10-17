import { expect, test, type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';
import { pickRecord } from './pickers';

// Captures the restore dialog at a wide and a narrow width for a review. It runs only with CAPTURE
// and CAPTURE_DIR set, so CI skips it.
const dir = process.env.CAPTURE_DIR;
test.skip(!process.env.CAPTURE || !dir, 'set CAPTURE and CAPTURE_DIR to write the screenshots');

const out = dir as string;

async function openDialog(page: Page, width: number): Promise<void> {
	mkdirSync(out, { recursive: true });
	await page.setViewportSize({ width, height: 860 });
	await page.addInitScript(() => {
		localStorage.setItem('restichub.mock.agents', 'sample');
		localStorage.setItem('restichub.mock.repositories', 'sample');
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
	await page
		.getByRole('link', { name: /^Открыть снимок / })
		.first()
		.click();
	const tree = page.getByRole('tree');
	await tree.getByRole('checkbox', { name: 'Отметить etc' }).check();
	await page.getByRole('button', { name: 'Восстановить…' }).click();
	await expect(page.getByRole('dialog')).toBeVisible();
	await page.waitForTimeout(300);
}

for (const width of [1280, 860]) {
	test(`capture the restore dialog at ${width} px`, async ({ page }) => {
		await openDialog(page, width);
		await page.screenshot({ path: `${out}/restore-dialog-${width}-ru.png` });

		const dialog = page.getByRole('dialog');
		await pickRecord(page, dialog.locator('#restore-agent'), 'win-fs01');
		await dialog.locator('#restore-target').fill('C:\\ProgramData\\ResticHub-Restore\\job-1');
		await page.waitForTimeout(300);
		await page.screenshot({ path: `${out}/restore-dialog-${width}-windows-ru.png` });
	});
}
