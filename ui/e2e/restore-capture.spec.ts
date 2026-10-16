import { expect, test, type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';

// Captures the Restore pages for a review: the snapshot list, the snapshot page, the restore dialog
// and the job page.
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

async function capture(
	page: Page,
	scheme: 'light' | 'dark',
	labels: Labels,
	suffix: string,
	width = 1440
) {
	mkdirSync(out, { recursive: true });
	await page.setViewportSize({ width, height: width < 600 ? 800 : width > 1600 ? 1080 : 900 });
	const size = width < 600 ? '-mobile' : width > 1600 ? '-1920' : '';
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

	const rows = page.getByRole('link', { name: labels.snapshot });
	await expect(rows.first()).toBeVisible();
	await page.waitForTimeout(300);
	await page.screenshot({
		path: `${out}/restore-snapshots-${scheme}${suffix}${size}.png`,
		fullPage: true
	});

	const listOverflow = await page.evaluate(
		() => document.documentElement.scrollWidth - document.documentElement.clientWidth
	);
	expect(listOverflow).toBeLessThanOrEqual(0);

	await rows.first().click();
	const tree = page.getByRole('tree');
	await tree.getByRole('button', { name: labels.expand('srv') }).click();
	await tree.getByRole('button', { name: labels.expand('www') }).click();
	await tree.getByRole('button', { name: labels.expand('data') }).click();
	await tree.getByRole('button', { name: labels.expand('home') }).click();
	await tree.getByRole('button', { name: labels.expand('anna') }).click();
	await tree.getByRole('checkbox', { name: labels.select('backup.sql') }).check();
	await tree.getByRole('checkbox', { name: labels.select('photo.jpg') }).check();
	await page.waitForTimeout(300);
	await page.screenshot({ path: `${out}/restore-snapshot-${scheme}${suffix}${size}.png` });
	if (width >= 1024) {
		// The tree scrolls inside its panel, the page does not.
		const extra = await page.evaluate(
			() => document.documentElement.scrollHeight - document.documentElement.clientHeight
		);
		expect(extra).toBeLessThanOrEqual(0);
	}
	await page.evaluate(() => window.scrollTo(0, 0));
	await page.waitForTimeout(200);
	await page.screenshot({ path: `${out}/restore-snapshot-top-${scheme}${suffix}${size}.png` });
	const overflow = await page.evaluate(
		() => document.documentElement.scrollWidth - document.documentElement.clientWidth
	);
	expect(overflow).toBeLessThanOrEqual(0);

	await page.getByRole('button', { name: labels.open }).click();
	const dialog = page.getByRole('dialog');
	await page.waitForTimeout(300);
	await page.screenshot({ path: `${out}/restore-dialog-${scheme}${suffix}${size}.png` });
	await dialog.getByLabel(labels.existing).selectOption({ index: 0 });
	await page.waitForTimeout(200);
	await page.screenshot({ path: `${out}/restore-dialog-replace-${scheme}${suffix}${size}.png` });
	await dialog.locator('button[type="submit"]').click();
	await expect(page.getByRole('heading', { level: 1, name: /^j_/ })).toBeVisible();
	await page.waitForTimeout(1500);
	await page.screenshot({
		path: `${out}/restore-job-${scheme}${suffix}${size}.png`,
		fullPage: true
	});
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

test('captures the Restore pages in Russian, light, 390 wide', async ({ page }) => {
	await capture(page, 'light', RU, '-ru', 390);
});

test('captures the Restore pages in Russian, light, 1920 wide', async ({ page }) => {
	await capture(page, 'light', RU, '-ru', 1920);
});
