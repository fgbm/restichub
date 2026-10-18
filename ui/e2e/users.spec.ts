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
	// The right and the state of the account are in the dangerous section only.
	const danger = page.locator('[data-section="danger"]');
	const revoke = danger.getByRole('button', { name: 'Remove the Platform Admin right' });
	await expect(revoke).toBeDisabled();
	await expect(revoke).toHaveAttribute('title', 'You cannot remove your own Platform Admin right');
	const disable = danger.getByRole('button', { name: 'Disable user' });
	await expect(disable).toBeDisabled();
	await expect(disable).toHaveAttribute('title', 'You cannot disable your own account');
	// A lower own role is allowed after a confirmation.
	await expect(page.getByLabel('Role in the organization acme')).toBeEnabled();
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

test('ac_au_10 a taken name or email marks only the field that the server names', async ({
	page
}) => {
	await signIn(page);
	await openPlatformUsers(page);
	await page.getByRole('button', { name: 'Create user' }).click();
	const dialog = page.getByRole('dialog');
	const username = dialog.locator('#platform-user-username');
	const email = dialog.locator('#platform-user-email');
	await username.fill('boris');
	await dialog.locator('#platform-user-display-name').fill('Boris Again');
	await dialog.locator('#platform-user-password').fill('a long enough password');
	await dialog.getByRole('button', { name: 'Create user' }).click();
	await expect(username).toHaveAttribute('aria-invalid', 'true');
	await expect(dialog.getByText('This user name is taken.')).toBeVisible();
	await expect(email).not.toHaveAttribute('aria-invalid', 'true');

	await username.fill('carol');
	await email.fill('anna@example.com');
	await dialog.getByRole('button', { name: 'Create user' }).click();
	await expect(email).toHaveAttribute('aria-invalid', 'true');
	await expect(dialog.getByText('This email is taken.')).toBeVisible();
	await expect(username).not.toHaveAttribute('aria-invalid', 'true');
});
