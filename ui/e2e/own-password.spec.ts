import { expect, test, type Page } from '@playwright/test';

// The user menu changes the password of the signed-in account.
async function openDialog(page: Page): Promise<void> {
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.getByRole('button', { name: 'Sign in', exact: true }).click();
	await expect(page.getByRole('navigation', { name: 'Primary navigation' })).toBeVisible();
	await page.locator('.rh-menu__trigger').click();
	await page.getByRole('menuitem', { name: 'Change password' }).click();
	await expect(page.getByRole('dialog', { name: 'Change password' })).toBeVisible();
}

test('the own password changes from the user menu', async ({ page }) => {
	await openDialog(page);
	const dialog = page.getByRole('dialog', { name: 'Change password' });
	await dialog.getByLabel('Current password').fill('secret');
	await dialog.getByLabel('New password').fill('a-new-long-password');
	await dialog.getByRole('button', { name: 'Change password' }).click();
	await expect(page.getByText('The password is changed.')).toBeVisible();
	await expect(dialog).toBeHidden();
});

test('a wrong current password shows on its field', async ({ page }) => {
	await openDialog(page);
	const dialog = page.getByRole('dialog', { name: 'Change password' });
	const current = dialog.getByLabel('Current password');
	await current.fill('wrong');
	await dialog.getByLabel('New password').fill('a-new-long-password');
	await dialog.getByRole('button', { name: 'Change password' }).click();
	await expect(dialog.getByText('The current password is not correct.')).toBeVisible();
	await expect(current).toHaveAttribute('aria-invalid', 'true');
	await expect(current).toBeFocused();
});
