import { expect, test, type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';

// Captures the section «Опасные действия» of the record pages for a review. It runs only with
// CAPTURE and CAPTURE_DIR set, so CI skips it (UI principles 1.7).
const dir = process.env.CAPTURE_DIR;
test.skip(!process.env.CAPTURE || !dir, 'set CAPTURE and CAPTURE_DIR to write the screenshots');
test.describe.configure({ mode: 'serial' });

const out = dir as string;

const NAV = 'Основная навигация';
const DANGER = 'Опасные действия';

async function shot(page: Page, name: string): Promise<void> {
	await page.evaluate(() =>
		document
			.querySelectorAll<HTMLButtonElement>('.toast button')
			.forEach((button) => button.click())
	);
	await page.waitForTimeout(250);
	await page.screenshot({ path: `${out}/${name}.png` });
}

/** The section and the save bar below it, from the start of the section. */
async function shotDanger(page: Page, name: string): Promise<void> {
	await page
		.locator('[data-section="danger"]')
		.evaluate((element) => element.scrollIntoView({ block: 'start' }));
	await page.waitForTimeout(300);
	await shot(page, name);
}

async function start(page: Page): Promise<void> {
	mkdirSync(out, { recursive: true });
	await page.setViewportSize({ width: 1440, height: 900 });
	await page.emulateMedia({ colorScheme: 'light' });
	await page.addInitScript(() => {
		localStorage.setItem('restichub.mock.agents', 'sample');
		localStorage.setItem('restichub.mock.profiles', 'sample');
		localStorage.setItem('restichub.mock.plans', 'sample');
		localStorage.setItem('restichub.mock.scripts', 'sample');
		localStorage.setItem('restichub.locale', 'ru');
	});
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.locator('form button[type="submit"]').click();
	await expect(page.getByRole('navigation', { name: NAV })).toBeVisible();
}

test('capture the dangerous section of the record pages in Russian', async ({ page }) => {
	await start(page);

	// The plan page.
	await page.getByRole('navigation', { name: NAV }).locator('a[href="/plans"]').click();
	await page
		.getByRole('list', { name: 'Планы' })
		.getByRole('link', { name: 'nightly-web', exact: true })
		.click();
	await expect(page.getByRole('button', { name: new RegExp(DANGER) })).toBeVisible();
	await page.mouse.wheel(0, 4000);
	await shot(page, 'plan-editor-danger-ru-light');

	// The page of an own script template.
	await page.locator('a[href="/settings"]').first().click();
	await page.locator('a[href="/settings/scripts/templates"]').click();
	await page.getByRole('link', { name: 'Remove old dumps', exact: true }).click();
	await expect(page.getByRole('button', { name: new RegExp(DANGER) })).toBeVisible();
	await shotDanger(page, 'script-template-danger-ru-light');

	// The page of an own job template, which starts as a copy of a built-in template.
	await page.locator('a[href="/settings"]').first().click();
	await page.locator('a[href="/settings/scripts/jobs"]').click();
	await page.getByRole('button', { name: 'Сделать копию' }).first().click();
	await page.getByRole('button', { name: 'Сохранить' }).click();
	await expect(page.getByRole('heading', { level: 1, name: 'Шаблоны заданий' })).toBeVisible();
	await page.getByRole('link', { name: /копия/ }).first().click();
	await expect(page.getByRole('button', { name: new RegExp(DANGER) })).toBeVisible();
	await shotDanger(page, 'job-template-danger-ru-light');

	// The page of an installation account.
	await page.getByRole('button', { name: /Сменить организацию/ }).click();
	await page.getByRole('menuitem', { name: 'Администрирование платформы' }).click();
	await page.getByRole('navigation', { name: NAV }).locator('a[href="/platform/users"]').click();
	await page.getByRole('button', { name: /^Отключены/ }).click();
	await page.getByRole('link', { name: 'Boris Ivanov', exact: true }).click();
	await expect(page.getByRole('button', { name: new RegExp(DANGER) })).toBeVisible();
	await page.mouse.wheel(0, 4000);
	await shot(page, 'platform-user-danger-ru-light');
});
