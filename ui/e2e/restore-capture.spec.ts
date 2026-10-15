import { expect, test, type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';

// Captures the Restore screen for a review: the snapshot list, the tree and the restore dialog.
// It runs only with CAPTURE and CAPTURE_DIR set, so CI skips it (FR-RS-01 to FR-RS-04).
const dir = process.env.CAPTURE_DIR;
test.skip(!process.env.CAPTURE || !dir, 'set CAPTURE and CAPTURE_DIR to write the screenshots');

const out = dir as string;

type Labels = {
	nav: string;
	restore: string;
	snapshot: RegExp;
	expand: (name: string) => string;
	select: (name: string) => string;
	open: string;
	target: string;
	host: string;
	existing: string;
};

const EN: Labels = {
	nav: 'Primary navigation',
	restore: 'Restore',
	snapshot: /^Open snapshot /,
	expand: (name) => `Expand ${name}`,
	select: (name) => `Select ${name}`,
	open: 'Restore…',
	target: 'Target path',
	host: 'Target host',
	existing: 'Existing files'
};

const RU: Labels = {
	nav: 'Основная навигация',
	restore: 'Восстановление',
	snapshot: /^Открыть снимок /,
	expand: (name) => `Раскрыть ${name}`,
	select: (name) => `Отметить ${name}`,
	open: 'Восстановить…',
	target: 'Целевой путь',
	host: 'Целевой хост',
	existing: 'Существующие файлы'
};

async function capture(page: Page, scheme: 'light' | 'dark', labels: Labels, suffix: string) {
	mkdirSync(out, { recursive: true });
	await page.setViewportSize({ width: 1440, height: 900 });
	await page.emulateMedia({ colorScheme: scheme });
	await page.addInitScript((value) => {
		localStorage.setItem('restichub.mock.agents', 'sample');
		localStorage.setItem('restichub.mock.repositories', 'sample');
		localStorage.setItem('restichub.theme', value);
	}, scheme);
	if (labels === RU) {
		await page.addInitScript(() => localStorage.setItem('restichub.locale', 'ru'));
	}
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.locator('form button[type="submit"]').click();
	await page
		.getByRole('navigation', { name: labels.nav })
		.getByRole('link', { name: labels.restore, exact: true })
		.click();

	const rows = page.getByRole('button', { name: labels.snapshot });
	await expect(rows.first()).toBeVisible();
	await page.waitForTimeout(300);
	await page.screenshot({ path: `${out}/restore-snapshots-${scheme}${suffix}.png` });

	await rows.first().click();
	const tree = page.getByRole('tree');
	await tree.getByRole('button', { name: labels.expand('srv') }).click();
	await tree.getByRole('button', { name: labels.expand('www') }).click();
	await tree.getByRole('button', { name: labels.expand('etc') }).click();
	await tree.getByRole('checkbox', { name: labels.select('data') }).check();
	await page.waitForTimeout(300);
	await page.screenshot({ path: `${out}/restore-tree-${scheme}${suffix}.png` });

	await page.getByRole('button', { name: labels.open }).click();
	const dialog = page.getByRole('dialog');
	await dialog.getByLabel(labels.host).selectOption({ label: 'srv-db-01 · linux' });
	await page.waitForTimeout(300);
	await page.screenshot({ path: `${out}/restore-dialog-${scheme}${suffix}.png` });
}

test('captures the Restore screen in English, light', async ({ page }) => {
	await capture(page, 'light', EN, '-en');
});

test('captures the Restore screen in English, dark', async ({ page }) => {
	await capture(page, 'dark', EN, '-en');
});

test('captures the Restore screen in Russian, light', async ({ page }) => {
	await capture(page, 'light', RU, '-ru');
});
