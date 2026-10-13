import { test } from '@playwright/test';
import { mkdirSync } from 'node:fs';

// Captures the screens for a review. It runs only with CAPTURE and CAPTURE_DIR set, so CI skips it.
const dir = process.env.CAPTURE_DIR;
test.skip(!process.env.CAPTURE || !dir, 'set CAPTURE and CAPTURE_DIR to write the screenshots');

test('capture the screens', async ({ page }) => {
	const out = dir as string;
	mkdirSync(out, { recursive: true });
	await page.setViewportSize({ width: 1280, height: 900 });

	await page.goto('/setup');
	await page.locator('#setup-token').fill('token');
	await page.locator('#setup-username').fill('admin');
	await page.locator('#setup-display-name').fill('Anna Petrova');
	await page.locator('#setup-password').fill('password1234');
	await page.locator('#setup-confirm').fill('password1234');
	await page.getByRole('button', { name: 'Create the Platform Admin' }).click();

	await page.locator('#signin-username').waitFor();
	await page.screenshot({ path: out + '/sign-in.png', fullPage: true });

	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.getByRole('button', { name: 'Sign in' }).click();
	await page.getByRole('navigation').waitFor();

	await page.goto('/users');
	await page.getByRole('heading', { level: 1 }).waitFor();
	await page.screenshot({ path: out + '/users.png', fullPage: true });

	await page.goto('/platform/tenants');
	await page.getByRole('heading', { level: 1 }).waitFor();
	await page.screenshot({ path: out + '/tenants.png', fullPage: true });
});
