import { expect, test, type Page } from '@playwright/test';

// The Secrets screen under Settings, Access: create, replace and delete (FR-RP-05, AC-RP-02).
async function openSecrets(page: Page): Promise<void> {
	await page.addInitScript(() => {
		localStorage.setItem('restichub.mock.agents', 'sample');
		localStorage.setItem('restichub.mock.profiles', 'sample');
		localStorage.setItem('restichub.mock.scripts', 'sample');
		localStorage.setItem('restichub.mock.repositories', 'sample');
	});
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.getByRole('button', { name: 'Sign in', exact: true }).click();
	await expect(page.getByRole('navigation', { name: 'Primary navigation' })).toBeVisible();
	await page.locator('a[href="/settings"]').first().click();
	await page.locator('a[href="/settings/access/secrets"]').click();
	await expect(page.getByRole('heading', { name: 'Secrets', exact: true })).toBeVisible();
}

test('a secret is created, its value is replaced and it is deleted', async ({ page }) => {
	await openSecrets(page);

	await page.getByRole('button', { name: 'Add secret' }).click();
	let dialog = page.getByRole('dialog');
	await dialog.getByLabel(/^Name/).fill('backup.token');
	const value = dialog.getByLabel(/^Value/);
	await expect(value).toHaveAttribute('type', 'password');
	await value.fill('first-value');
	await dialog.getByRole('button', { name: 'Add secret' }).click();
	await expect(page.getByText('The secret is added.')).toBeVisible();
	const row = page.locator('li.item').filter({ hasText: 'backup.token' });
	await expect(row).toBeVisible();
	await expect(page.getByText('first-value')).toHaveCount(0);

	await row.getByRole('button', { name: 'Replace value' }).click();
	dialog = page.getByRole('dialog');
	await expect(dialog.getByText(/next delivery/)).toBeVisible();
	await dialog.getByLabel(/^New value/).fill('second-value');
	await dialog.getByRole('button', { name: 'Replace value' }).click();
	await expect(page.getByText('The value is replaced.')).toBeVisible();
	await expect(page.getByText('second-value')).toHaveCount(0);

	await row.getByRole('button', { name: 'Delete' }).click();
	await page.getByRole('dialog').getByRole('button', { name: 'Delete' }).click();
	await expect(page.getByText('The secret is deleted.')).toBeVisible();
	await expect(page.locator('li.item').filter({ hasText: 'backup.token' })).toHaveCount(0);
});

test('a secret in use cannot be deleted and its details name its users', async ({ page }) => {
	await openSecrets(page);
	const row = page.locator('li.item').filter({ hasText: 'db.password' });
	// An item has no link to another entity; the details panel names the users with links.
	await expect(row.getByText('used by 1 profile')).toBeVisible();
	await expect(row.getByRole('button', { name: 'Delete' })).toBeDisabled();

	await row.getByRole('button', { name: 'db.password' }).click();
	const panel = page.getByRole('dialog');
	await expect(panel.getByRole('link', { name: 'db-nightly' })).toBeVisible();
	await page.keyboard.press('Escape');
	await expect(row).toBeVisible();
});

test('the password of a repository offers no replace', async ({ page }) => {
	await openSecrets(page);
	const row = page.locator('li.item').filter({ hasText: 'primary-s3.password' });
	await expect(row.getByRole('button', { name: 'Delete' })).toBeVisible();
	await expect(row.getByRole('button', { name: 'Replace value' })).toHaveCount(0);
});
