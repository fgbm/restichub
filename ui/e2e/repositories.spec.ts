import { expect, test, type Page } from '@playwright/test';

// The Repositories screen lists the repositories of the organization and creates, edits and
// deletes one in a centered dialog (FR-RP-01, FR-RP-05).
async function signIn(page: Page): Promise<void> {
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.getByRole('button', { name: 'Sign in', exact: true }).click();
	await expect(page.getByRole('navigation', { name: 'Primary navigation' })).toBeVisible();
}

test('the repositories screen creates, edits and deletes a repository', async ({ page }) => {
	await signIn(page);
	await page
		.getByRole('navigation', { name: 'Primary navigation' })
		.getByRole('link', { name: 'Repositories', exact: true })
		.click();

	await expect(page.getByRole('heading', { level: 1, name: 'Repositories' })).toBeVisible();
	await expect(page).toHaveTitle('Repositories · ResticHub');
	// The mock holds no repository, so the empty state shows.
	await expect(page.getByText('No repositories yet')).toBeVisible();

	await page.getByRole('button', { name: 'Add repository' }).click();
	const dialog = page.getByRole('dialog', { name: 'Add a repository' });
	await expect(dialog).toBeVisible();
	await page.locator('#repo-name').fill('s3-prod');
	await page.locator('#repo-endpoint').fill('https://s3.example.net');
	await page.locator('#repo-bucket').fill('backups-prod');
	await page.locator('#repo-password').fill('repo-password');
	await page.locator('#repo-s3-access-key').fill('access-key');
	await page.locator('#repo-s3-secret-key').fill('secret-key');
	await dialog.getByRole('button', { name: 'Save' }).click();

	await expect(page.getByText('The repository is created.')).toBeVisible();
	await expect(page.getByText('s3-prod')).toBeVisible();
	await expect(page.getByText('Not initialized')).toBeVisible();
	await expect(page.getByText('s3://backups-prod')).toBeVisible();

	// Edit: the name changes and the secret fields stay empty.
	await page.getByRole('button', { name: 'More actions for s3-prod' }).click();
	await page.getByRole('menuitem', { name: 'Edit' }).click();
	const edit = page.getByRole('dialog', { name: 'Edit the repository' });
	await expect(edit).toBeVisible();
	await expect(page.locator('#repo-password')).toHaveValue('');
	await page.locator('#repo-name').fill('s3-prod-renamed');
	await edit.getByRole('button', { name: 'Save' }).click();
	await expect(page.getByText('The repository is updated.')).toBeVisible();
	await expect(page.getByText('s3-prod-renamed')).toBeVisible();

	// Delete.
	await page.getByRole('button', { name: 'More actions for s3-prod-renamed' }).click();
	await page.getByRole('menuitem', { name: 'Delete repository' }).click();
	await page.getByRole('button', { name: 'Delete repository', exact: true }).click();
	await expect(page.getByText('The repository is deleted.')).toBeVisible();
	await expect(page.getByText('No repositories yet')).toBeVisible();
});
