import { expect, test, type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';

// Captures the lists with the item fields that the server adds, for a review against the list
// mockups. It runs only with CAPTURE and CAPTURE_DIR set, so CI skips it.
const dir = process.env.CAPTURE_DIR;
test.skip(!process.env.CAPTURE || !dir, 'set CAPTURE and CAPTURE_DIR to write the screenshots');

const out = dir as string;

const LISTS: [string, string][] = [
	['/hosts', 'hosts'],
	['/repositories', 'repositories'],
	['/plans', 'plans'],
	['/jobs', 'jobs'],
	['/audit', 'audit'],
	['/settings/access/users', 'users'],
	['/settings/notifications/channels', 'channels']
];

async function signIn(page: Page): Promise<void> {
	mkdirSync(out, { recursive: true });
	await page.setViewportSize({ width: 1440, height: 900 });
	await page.emulateMedia({ colorScheme: 'light' });
	await page.addInitScript(() => {
		for (const key of ['agents', 'repositories', 'profiles', 'plans', 'alerts', 'scripts']) {
			localStorage.setItem(`restichub.mock.${key}`, 'sample');
		}
		localStorage.setItem('restichub.theme', 'light');
		localStorage.setItem('restichub.locale', 'ru');
	});
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.locator('form button[type="submit"]').click();
	await expect(page.getByRole('navigation', { name: 'Основная навигация' })).toBeVisible();
}

async function open(page: Page, path: string): Promise<void> {
	// A reload drops the mock session, so the capture follows the links of the shell.
	const nav = page.getByRole('navigation', { name: 'Основная навигация' });
	if (path.startsWith('/settings/')) await nav.locator('a[href="/settings"]').click();
	await page.locator(`a[href="${path}"]:visible`).first().click();
	await page.getByRole('heading', { level: 1 }).waitFor();
	await page.waitForTimeout(500);
}

test('captures the lists with the server item fields at 1440', async ({ page }) => {
	test.setTimeout(300_000);
	await signIn(page);
	for (const [path, name] of LISTS) {
		await open(page, path);
		await page.screenshot({ path: `${out}/${name}-1440.png`, fullPage: true });
	}
	await page.getByRole('button', { name: /Сменить организацию/ }).click();
	await page.getByRole('menuitem', { name: 'Администрирование платформы' }).click();
	await page.getByRole('heading', { level: 1 }).waitFor();
	await page.waitForTimeout(500);
	await page.screenshot({ path: `${out}/platform-tenants-1440.png`, fullPage: true });
});
