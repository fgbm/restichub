import { expect, test, type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';

// Captures the user record pages of the platform and of the organization for a review against
// the mockup. It runs only with CAPTURE and CAPTURE_DIR set, so CI skips it.
const dir = process.env.CAPTURE_DIR;
test.skip(!process.env.CAPTURE || !dir, 'set CAPTURE and CAPTURE_DIR to write the screenshots');

const out = dir as string;

async function signIn(page: Page, width: number): Promise<void> {
	mkdirSync(out, { recursive: true });
	await page.setViewportSize({ width, height: width < 600 ? 844 : 900 });
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
}

async function shot(page: Page, name: string, width: number, full = true): Promise<void> {
	await page.waitForTimeout(400);
	await page.screenshot({ path: `${out}/${name}-${width}.png`, fullPage: full });
}

async function platformUser(page: Page, name: string): Promise<void> {
	await page.getByRole('link', { name, exact: true }).click();
	await page.getByRole('heading', { level: 1, name }).waitFor();
}

for (const width of [1440, 390]) {
	test(`captures the user pages at ${width}`, async ({ page }) => {
		test.setTimeout(300_000);
		await signIn(page, width);

		await page.getByRole('button', { name: /Сменить организацию/ }).click();
		await page.getByRole('menuitem', { name: 'Администрирование платформы' }).click();
		const nav = page.getByRole('navigation', { name: 'Основная навигация' });
		await nav.getByRole('link', { name: 'Пользователи', exact: true }).click();
		await page.getByRole('heading', { level: 1, name: 'Пользователи' }).waitFor();
		await shot(page, 'platform-list', width);
		await page.getByRole('button', { name: 'Другие действия: Anna Petrova' }).click();
		await shot(page, 'platform-list-menu-self', width, false);
		await page.keyboard.press('Escape');

		await platformUser(page, 'Anna Petrova');
		await shot(page, 'platform-self', width);
		await page.getByRole('button', { name: 'Другие действия: Anna Petrova' }).click();
		await shot(page, 'platform-self-menu', width, false);
		await page.keyboard.press('Escape');

		await page.goBack();
		await page.getByRole('heading', { level: 1, name: 'Пользователи' }).waitFor();
		await page.getByRole('button', { name: /^Отключены/ }).click();
		await platformUser(page, 'Boris Ivanov');
		await shot(page, 'platform-other-empty', width);
		await page.locator('#membership-add-tenant').selectOption({ label: 'globex' });
		await shot(page, 'platform-other-add', width);
		await page.getByRole('button', { name: 'Добавить организацию' }).click();
		await page.getByText('Членство добавлено.').waitFor();
		await shot(page, 'platform-other', width);
		await page.getByRole('button', { name: 'Другие действия: Boris Ivanov' }).click();
		await shot(page, 'platform-other-menu', width, false);
		await page.keyboard.press('Escape');
		await page.getByRole('button', { name: 'Убрать из организации globex' }).click();
		await shot(page, 'platform-remove-dialog', width, false);
		await page.keyboard.press('Escape');
		await page.getByRole('button', { name: 'Сбросить пароль' }).click();
		await shot(page, 'platform-reset-dialog', width, false);
		await page.keyboard.press('Escape');

		await page
			.getByRole('button', { name: /Сменить организацию|Администрирование платформы/ })
			.click();
		await page.getByRole('menuitemradio', { name: /acme/ }).click();
		await nav.locator('a[href="/settings"]').click();
		await page.locator('a[href="/settings/access/users"]:visible').first().click();
		await page.getByRole('heading', { level: 1, name: 'Пользователи' }).waitFor();
		await shot(page, 'org-list', width);
		await page.getByRole('link', { name: 'Boris Ivanov', exact: true }).click();
		await page.getByRole('heading', { level: 1, name: 'Boris Ivanov' }).waitFor();
		await shot(page, 'org-other', width);
		await page.getByRole('button', { name: 'Другие действия: Boris Ivanov' }).click();
		await shot(page, 'org-other-menu', width, false);
		await page.keyboard.press('Escape');
		await page.goBack();
		await page.getByRole('link', { name: 'Anna Petrova', exact: true }).click();
		await page.getByRole('heading', { level: 1, name: 'Anna Petrova' }).waitFor();
		await shot(page, 'org-self', width);
	});
}
