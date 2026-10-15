import { expect, test, type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';

// Captures the screens for a review. It runs only with CAPTURE and CAPTURE_DIR set, so CI skips it.
const dir = process.env.CAPTURE_DIR;
test.skip(!process.env.CAPTURE || !dir, 'set CAPTURE and CAPTURE_DIR to write the screenshots');
test.describe.configure({ mode: 'serial' });

const out = dir as string;

async function signIn(page: Page): Promise<void> {
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.getByRole('button', { name: 'Sign in', exact: true }).click();
	await expect(page.getByRole('navigation', { name: 'Primary navigation' })).toBeVisible();
}

// The mock keeps the session in memory, so the capture follows links instead of page loads.
async function open(page: Page, name: string): Promise<void> {
	await page
		.getByRole('navigation', { name: 'Primary navigation' })
		.getByRole('link', { name, exact: true })
		.click();
	await page.getByRole('heading', { level: 1 }).waitFor();
}

async function shot(page: Page, name: string, scheme: string): Promise<void> {
	// A toast of an earlier step would cover the screen under review.
	await page.evaluate(() =>
		document
			.querySelectorAll<HTMLButtonElement>('.toast button')
			.forEach((button) => button.click())
	);
	await page.waitForTimeout(250);
	await page.screenshot({ path: `${out}/${name}-${scheme}-app.png` });
}

for (const scheme of ['light', 'dark'] as const) {
	test(`capture the screens in the ${scheme} theme`, async ({ page }) => {
		mkdirSync(out, { recursive: true });
		await page.setViewportSize({ width: 1440, height: 900 });
		await page.emulateMedia({ colorScheme: scheme });
		await page.addInitScript(() => localStorage.setItem('restichub.mock.agents', 'sample'));
		await signIn(page);
		await page.getByText('Get started').waitFor();
		await shot(page, 'shell', scheme);

		await open(page, 'Hosts');
		await page.getByText('win-fs01').waitFor();
		await shot(page, 'hosts-data', scheme);

		await page.getByRole('button', { name: /Switch organization/ }).click();
		await page.getByRole('menu').waitFor();
		await shot(page, 'switcher-open', scheme);
		await page.keyboard.press('Escape');

		await page.locator('.rh-menu__trigger').filter({ hasText: 'Anna' }).click();
		await page.getByRole('menuitem', { name: 'Sign out' }).waitFor();
		await shot(page, 'account-menu-open', scheme);
		await page.keyboard.press('Escape');

		await page.getByRole('button', { name: 'Add host' }).click();
		await page.locator('#host-labels').fill('web');
		await page.getByRole('button', { name: 'Create token' }).click();
		await page.locator('.copy-field').first().waitFor();
		await shot(page, 'add-host-dialog', scheme);
		await page.keyboard.press('Escape');

		await open(page, 'Settings');
		await page.getByRole('heading', { name: 'Roles' }).waitFor();
		await shot(page, 'settings-access-users', scheme);

		await page.getByRole('button', { name: 'Create user' }).click();
		await page.locator('#user-username').fill('operator');
		await shot(page, 'create-dialog', scheme);
		await page.keyboard.press('Escape');

		await page
			.getByRole('navigation', { name: 'Settings sections' })
			.getByRole('link', { name: 'API tokens' })
			.click();
		await page.getByRole('heading', { level: 1, name: 'API tokens' }).waitFor();
		await page.locator('tbody tr').first().waitFor();
		await shot(page, 'settings-access-api-tokens', scheme);

		await page.getByRole('button', { name: 'Create token' }).click();
		await page.locator('#token-name').fill('ansible');
		await page.getByRole('dialog').getByRole('button', { name: 'Create token' }).click();
		await page.getByRole('dialog', { name: 'Copy the token value' }).waitFor();
		await shot(page, 'token-value-dialog', scheme);
		await page.keyboard.press('Escape');

		await page.getByRole('button', { name: /Switch organization/ }).click();
		await page.getByRole('menuitem', { name: 'Platform administration' }).click();
		await page.getByRole('heading', { level: 1 }).waitFor();
		await page.getByText('initech').waitFor();
		await shot(page, 'platform-organizations', scheme);

		await page
			.getByRole('row', { name: /initech/ })
			.getByRole('button', { name: 'Delete' })
			.click();
		await page.locator('#tenant-delete-confirm').waitFor();
		await shot(page, 'delete-org-dialog', scheme);
		await page.keyboard.press('Escape');

		await open(page, 'Users');
		await page.locator('tbody tr').first().waitFor();
		await shot(page, 'platform-users', scheme);

		await open(page, 'Audit');
		await page.locator('tbody tr').first().waitFor();
		await shot(page, 'platform-audit', scheme);

		await page.getByRole('button', { name: /Switch organization/ }).click();
		await page.getByRole('menu').waitFor();
		await shot(page, 'switcher-platform-mode', scheme);
		await page.getByRole('menuitemradio', { name: /acme/ }).click();
		await page.getByText('Get started').waitFor();
		await shot(page, 'back-in-organization', scheme);
	});

	test(`capture the empty hosts screen in the ${scheme} theme`, async ({ page }) => {
		mkdirSync(out, { recursive: true });
		await page.setViewportSize({ width: 1440, height: 900 });
		await page.emulateMedia({ colorScheme: scheme });
		await signIn(page);
		await open(page, 'Hosts');
		await page.getByText('No hosts yet').waitFor();
		await shot(page, 'hosts-empty', scheme);
	});
}

test('capture the shell and the access screen in Russian', async ({ page }) => {
	mkdirSync(out, { recursive: true });
	await page.setViewportSize({ width: 1440, height: 900 });
	await page.emulateMedia({ colorScheme: 'light' });
	await page.addInitScript(() => localStorage.setItem('restichub.locale', 'ru'));
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.locator('form button[type="submit"]').click();
	const nav = page.getByRole('navigation', { name: 'Основная навигация' });
	await expect(nav).toBeVisible();
	await shot(page, 'shell-ru', 'light');
	await nav.getByRole('link', { name: 'Настройки', exact: true }).click();
	await page.getByRole('heading', { level: 1, name: 'Пользователи' }).waitFor();
	await shot(page, 'settings-access-users-ru', 'light');
});
