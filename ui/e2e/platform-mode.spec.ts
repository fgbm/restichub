import { expect, test, type Page } from '@playwright/test';

// The switcher entry opens the platform screens with their own tab set (ADR-0029).
async function signIn(page: Page): Promise<void> {
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.getByRole('button', { name: 'Sign in', exact: true }).click();
	await expect(page.getByRole('navigation', { name: 'Primary navigation' })).toBeVisible();
}

async function tabNames(page: Page): Promise<string[]> {
	const tabs = page.getByRole('navigation', { name: 'Primary navigation' }).getByRole('link');
	return (await tabs.allTextContents()).map((text) => text.trim());
}

test('the switcher entry enters the platform mode and an organization leaves it', async ({
	page
}) => {
	await signIn(page);
	const switcher = page.getByRole('button', { name: 'Switch organization' });
	await expect(switcher).toHaveText('acme');
	expect(await tabNames(page)).toEqual(['Overview', 'Hosts', 'Audit', 'Settings']);

	await switcher.click();
	await page.getByRole('menuitem', { name: 'Platform administration' }).click();

	await expect(page).toHaveURL(/\/platform\/tenants$/);
	await expect(page).toHaveTitle('Organizations · Platform administration · ResticHub');
	await expect(switcher).toHaveText('Platform administration');
	await expect(switcher.locator('svg[data-icon="shield"]')).toBeVisible();
	const breadcrumb = page.getByRole('navigation', { name: 'Breadcrumb' });
	await expect(breadcrumb.locator('[aria-current="page"]')).toHaveText('Organizations');
	expect(await tabNames(page)).toEqual(['Organizations', 'Users', 'Audit']);

	const nav = page.getByRole('navigation', { name: 'Primary navigation' });
	await nav.getByRole('link', { name: 'Users', exact: true }).click();
	await expect(page).toHaveURL(/\/platform\/users$/);
	await expect(page).toHaveTitle('Users · Platform administration · ResticHub');
	await expect(page.getByRole('heading', { level: 1, name: 'Users' })).toBeVisible();

	await nav.getByRole('link', { name: 'Audit', exact: true }).click();
	await expect(page).toHaveURL(/\/platform\/audit$/);
	await expect(page).toHaveTitle('Audit · Platform administration · ResticHub');
	expect(await tabNames(page)).toEqual(['Organizations', 'Users', 'Audit']);

	await switcher.click();
	await page.getByRole('menuitemradio', { name: /globex/ }).click();

	await expect(page).toHaveURL(/\/$/);
	await expect(switcher).toHaveText('globex');
	expect(await tabNames(page)).toEqual(['Overview', 'Hosts', 'Audit']);
});

test('the current organization leaves the platform mode for its overview', async ({ page }) => {
	await signIn(page);
	const switcher = page.getByRole('button', { name: 'Switch organization' });
	await switcher.click();
	await page.getByRole('menuitem', { name: 'Platform administration' }).click();
	await expect(page).toHaveURL(/\/platform\/tenants$/);

	await switcher.click();
	await page.getByRole('menuitemradio', { name: /acme/ }).click();

	await expect(page).toHaveURL(/\/$/);
	await expect(switcher).toHaveText('acme');
	await expect(page).toHaveTitle(/^Overview · ResticHub$/);
	expect(await tabNames(page)).toEqual(['Overview', 'Hosts', 'Audit', 'Settings']);
});

// ADR-0029: the Platform Admin right without a membership leaves no organization screen to open.
test('a Platform Admin without an organization reaches the platform mode on every organization route', async ({
	page
}) => {
	await page.addInitScript(() =>
		localStorage.setItem('restichub.mock.session', 'platform-admin-no-org')
	);

	await signIn(page);
	await expect(page).toHaveURL(/\/platform\/tenants$/);

	await page.goto('/hosts');
	await expect(page).toHaveURL(/\/platform\/tenants$/);
	await expect(page.getByRole('heading', { level: 1, name: 'Organizations' })).toBeVisible();
	expect(await page.locator('.toast').count()).toBe(0);
});
