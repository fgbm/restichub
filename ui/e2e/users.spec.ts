import { expect, test, type Page } from '@playwright/test';

// The user record pages of the platform and of the organization (ADR-0028, ADR-0029).
async function signIn(page: Page): Promise<void> {
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.getByRole('button', { name: 'Sign in', exact: true }).click();
	await expect(page.getByRole('navigation', { name: 'Primary navigation' })).toBeVisible();
}

async function openPlatformUsers(page: Page): Promise<void> {
	await page.getByRole('button', { name: /Switch organization/ }).click();
	await page.getByRole('menuitem', { name: 'Platform administration' }).click();
	await page
		.getByRole('navigation', { name: 'Primary navigation' })
		.getByRole('link', { name: 'Users', exact: true })
		.click();
	await expect(page.getByRole('heading', { level: 1, name: 'Users' })).toBeVisible();
}

test('a platform account opens as a page and its memberships change there', async ({ page }) => {
	await signIn(page);
	await openPlatformUsers(page);
	await page.getByRole('button', { name: /^Disabled/ }).click();
	await page.getByRole('link', { name: 'Boris Ivanov', exact: true }).click();
	await expect(page).toHaveURL(/\/platform\/users\/pu_03$/);
	await expect(page.getByRole('heading', { level: 1, name: 'Boris Ivanov' })).toBeVisible();
	const breadcrumb = page.getByRole('navigation', { name: 'Breadcrumb' });
	await expect(breadcrumb.getByRole('link', { name: 'Users' })).toBeVisible();
	await expect(page.getByText('The user is not a member of an organization.')).toBeVisible();

	const add = page.getByRole('button', { name: 'Add an organization' });
	await expect(add).toBeDisabled();
	const organization = page.locator('#membership-add-tenant');
	// The suspended organization is not a choice.
	await expect(organization.locator('option', { hasText: 'initech' })).toHaveCount(0);
	await organization.selectOption({ label: 'globex' });
	await add.click();
	await expect(page.getByText('The membership is added.')).toBeVisible();
	await expect(organization.locator('option', { hasText: 'globex' })).toHaveCount(0);

	const role = page.getByLabel('Role in the organization globex');
	await role.selectOption('operator');
	await expect(page.getByText('The role is changed.')).toBeVisible();

	await page.getByRole('button', { name: 'Remove from the organization globex' }).click();
	const dialog = page.getByRole('dialog', { name: 'Remove from the organization?' });
	await dialog.getByRole('button', { name: 'Remove' }).click();
	await expect(page.getByText('The membership is removed.')).toBeVisible();
	await expect(page.getByText('The user is not a member of an organization.')).toBeVisible();
});

test('the own platform account disables the actions that the API refuses', async ({ page }) => {
	await signIn(page);
	await openPlatformUsers(page);
	await page.getByRole('link', { name: 'Anna Petrova', exact: true }).click();
	await expect(page.getByRole('heading', { level: 1, name: 'Anna Petrova' })).toBeVisible();
	await expect(page.getByRole('button', { name: 'Reset password' })).toBeDisabled();
	await expect(
		page.getByRole('button', { name: 'Remove from the organization acme' })
	).toBeDisabled();
	await page.getByRole('button', { name: 'More actions for Anna Petrova' }).click();
	await expect(page.getByRole('menuitem', { name: /Disable user/ })).toContainText(
		'This is your account'
	);
});

test('an organization member opens as a page with its role', async ({ page }) => {
	await signIn(page);
	await page.locator('a[href="/settings"]').first().click();
	await page.locator('a[href="/settings/access/users"]').click();
	await expect(page.getByRole('heading', { level: 1, name: 'Users' })).toBeVisible();
	await expect(page.getByRole('heading', { name: 'Roles' })).toHaveCount(0);
	await page.getByRole('link', { name: 'Boris Ivanov', exact: true }).click();
	await expect(page).toHaveURL(/\/settings\/access\/users\/u_02$/);
	await expect(page.getByRole('heading', { level: 1, name: 'Boris Ivanov' })).toBeVisible();
	const role = page.getByLabel('Role', { exact: true });
	await expect(role).toHaveValue('operator');
	await role.selectOption('viewer');
	await expect(page.getByText('The role is changed.')).toBeVisible();
	await expect(page.getByText('Read only, including the organization audit log.')).toBeVisible();
});
