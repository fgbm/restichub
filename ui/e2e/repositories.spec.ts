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
	// The password field is create-only: the password cannot be replaced until FR-RP-06.
	await expect(page.locator('#repo-password')).toHaveCount(0);
	await expect(edit.getByText('cannot be replaced')).toBeVisible();
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

test('the repository init dialog picks a host and shows the job result', async ({ page }) => {
	await page.addInitScript(() => localStorage.setItem('restichub.mock.agents', 'sample'));
	await signIn(page);
	await page
		.getByRole('navigation', { name: 'Primary navigation' })
		.getByRole('link', { name: 'Repositories', exact: true })
		.click();
	await expect(page.getByRole('heading', { level: 1, name: 'Repositories' })).toBeVisible();

	// Create a repository with a maintenance host.
	await page.getByRole('button', { name: 'Add repository' }).click();
	await page.locator('#repo-name').fill('repo-a');
	await page.locator('#repo-endpoint').fill('https://s3.example.net');
	await page.locator('#repo-bucket').fill('backups-prod');
	await page.locator('#repo-password').fill('repo-password');
	await page.locator('#repo-s3-access-key').fill('access-key');
	await page.locator('#repo-s3-secret-key').fill('secret-key');
	await page.locator('#repo-agent').selectOption({ label: 'nas-backup' });
	await page.getByRole('dialog').getByRole('button', { name: 'Save' }).click();
	await expect(page.getByText('Not initialized')).toBeVisible();

	// Initialize: the dialog warns about the password copy and shows the job result (FR-RP-02).
	await page.getByRole('button', { name: 'More actions for repo-a' }).click();
	await page.getByRole('menuitem', { name: 'Initialize' }).click();
	const dialog = page.getByRole('dialog', { name: 'Initialize the repository?' });
	await expect(dialog).toBeVisible();
	await expect(
		dialog.getByText('Keep a copy of the repository password outside ResticHub.')
	).toBeVisible();
	await expect(page.locator('#job-host')).not.toHaveValue('');
	await dialog.getByRole('button', { name: 'Start' }).click();
	await expect(dialog.getByText('The repository is ready.')).toBeVisible({ timeout: 10_000 });
	await page.keyboard.press('Escape');
	// The status badge updates after the job (FR-RP-02).
	await expect(page.getByText('Initialized', { exact: true })).toBeVisible();

	// An initialized repository offers no Initialize action (HIGH 1).
	await page.getByRole('button', { name: 'More actions for repo-a' }).click();
	await expect(page.getByRole('menuitem', { name: 'Test access' })).toBeVisible();
	await expect(page.getByRole('menuitem', { name: 'Initialize' })).toHaveCount(0);
	await page.keyboard.press('Escape');
});
