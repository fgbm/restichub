import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';

// The check covers the WCAG 2 A and AA rules on the screens of the mock build, in both themes.
// A violation is a real defect of a component or of a screen; do not silence a rule here.

// The rules are engine independent, so one engine is enough and the run stays short.
test.skip(({ browserName }) => browserName !== 'chromium', 'axe runs in Chromium only');
test.setTimeout(90_000);

async function scan(page: Page): Promise<string[]> {
	const result = await new AxeBuilder({ page })
		.withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa'])
		.analyze();
	return result.violations.map(
		(violation) =>
			`${violation.id}: ${violation.nodes.map((node) => node.target.join(' ')).join(', ')}`
	);
}

async function signIn(page: Page): Promise<void> {
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.getByRole('button', { name: 'Sign in', exact: true }).click();
	await expect(page.getByRole('navigation', { name: 'Primary navigation' })).toBeVisible();
}

async function open(page: Page, name: string): Promise<void> {
	await page
		.getByRole('navigation', { name: 'Primary navigation' })
		.getByRole('link', { name, exact: true })
		.click();
	await page.getByRole('heading', { level: 1 }).waitFor();
}

for (const scheme of ['light', 'dark'] as const) {
	test.describe(`a11y in the ${scheme} theme`, () => {
		test.use({ colorScheme: scheme });

		test('the sign-in screen has no axe violation', async ({ page }) => {
			await page.goto('/sign-in');
			await expect(page.getByRole('heading', { level: 1, name: 'Sign in' })).toBeVisible();
			expect(await scan(page)).toEqual([]);
		});

		test('the shell screens have no axe violation', async ({ page }) => {
			await signIn(page);
			expect(await scan(page)).toEqual([]);
			for (const name of ['Hosts', 'Audit', 'Settings']) {
				await open(page, name);
				expect(await scan(page), name).toEqual([]);
			}
		});

		test('the About screen has no axe violation', async ({ page }) => {
			await signIn(page);
			// The mock build keeps the session in memory, so the screen opens from the account menu.
			await page.locator('.rh-menu__trigger').click();
			await page.getByRole('menuitem', { name: 'About ResticHub' }).click();
			await expect(page.getByRole('heading', { level: 1, name: 'About ResticHub' })).toBeVisible();
			expect(await scan(page)).toEqual([]);
		});

		test('the open menu and dialog have no axe violation', async ({ page }) => {
			await signIn(page);
			await page.locator('.rh-menu__trigger').click();
			await expect(page.getByRole('menuitem', { name: 'Sign out' })).toBeVisible();
			expect(await scan(page)).toEqual([]);
			await page.keyboard.press('Escape');

			await open(page, 'Settings');
			await page
				.getByRole('navigation', { name: 'Settings sections' })
				.getByRole('link', { name: 'API tokens' })
				.click();
			await page.getByRole('button', { name: 'Create token' }).click();
			await expect(page.getByRole('dialog')).toBeVisible();
			expect(await scan(page)).toEqual([]);
		});

		test('the component gallery has no axe violation', async ({ page }) => {
			await page.goto('/components');
			await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
			expect(await scan(page)).toEqual([]);
		});
	});
}
