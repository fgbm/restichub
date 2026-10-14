import { expect, test, type Page } from '@playwright/test';

// The Profiles screen lists the profiles of the organization, creates, edits and deletes one, and
// shows the compiled steps of the preview (FR-PF-01, FR-PF-04, FR-PF-05).
async function signIn(page: Page): Promise<void> {
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.getByRole('button', { name: 'Sign in', exact: true }).click();
	await expect(page.getByRole('navigation', { name: 'Primary navigation' })).toBeVisible();
}

// A profile needs a repository. The mock holds no repository, so the test creates one first.
async function createRepository(page: Page): Promise<void> {
	await page
		.getByRole('navigation', { name: 'Primary navigation' })
		.getByRole('link', { name: 'Repositories', exact: true })
		.click();
	await page.getByRole('button', { name: 'Add repository' }).click();
	await page.locator('#repo-name').fill('s3-prod');
	await page.locator('#repo-endpoint').fill('https://s3.example.net');
	await page.locator('#repo-bucket').fill('backups-prod');
	await page.locator('#repo-password').fill('repo-password');
	await page.locator('#repo-s3-access-key').fill('access-key');
	await page.locator('#repo-s3-secret-key').fill('secret-key');
	await page
		.getByRole('dialog', { name: 'Add a repository' })
		.getByRole('button', { name: 'Save' })
		.click();
	await expect(page.getByText('The repository is created.')).toBeVisible();
}

test('the profiles screen creates, previews, edits and deletes a profile', async ({ page }) => {
	// One host reports its facts, so the preview has a target.
	await page.addInitScript(() => localStorage.setItem('restichub.mock.agents', 'sample'));
	await signIn(page);
	await createRepository(page);

	await page
		.getByRole('navigation', { name: 'Primary navigation' })
		.getByRole('link', { name: 'Profiles', exact: true })
		.click();
	await expect(page.getByRole('heading', { level: 1, name: 'Profiles' })).toBeVisible();
	await expect(page).toHaveTitle('Profiles · ResticHub');
	await expect(page.getByText('No profiles yet')).toBeVisible();

	await page.getByRole('button', { name: 'Add profile' }).click();
	const dialog = page.getByRole('dialog', { name: 'Add a profile' });
	await expect(dialog).toBeVisible();
	await page.locator('#profile-name').fill('web-daily');
	await page.locator('#profile-paths-all').fill('/srv');
	await page.locator('#profile-paths-linux').fill('/etc');
	await page.locator('#profile-tags').fill('daily');
	await dialog.getByRole('button', { name: 'Save' }).click();

	await expect(page.getByText('The profile is created.')).toBeVisible();
	await expect(page.getByText('web-daily')).toBeVisible();

	// The preview shows the steps and the command line with masked secrets.
	await page.getByRole('button', { name: 'More actions for web-daily' }).click();
	await page.getByRole('menuitem', { name: 'Preview' }).click();
	await expect(page.getByRole('dialog', { name: 'Preview' })).toBeVisible();
	await page.locator('#preview-host').selectOption({ label: 'win-fs01' });
	await expect(page.getByText('restic backup', { exact: false })).toBeVisible();
	await expect(page.getByText('***', { exact: false }).first()).toBeVisible();
	await page.keyboard.press('Escape');

	// Edit: the name changes.
	await page.getByRole('button', { name: 'More actions for web-daily' }).click();
	await page.getByRole('menuitem', { name: 'Edit' }).click();
	const edit = page.getByRole('dialog', { name: 'Edit the profile' });
	await expect(edit).toBeVisible();
	await page.locator('#profile-name').fill('web-renamed');
	await edit.getByRole('button', { name: 'Save' }).click();
	await expect(page.getByText('The profile is updated.')).toBeVisible();
	await expect(page.getByText('web-renamed')).toBeVisible();

	// Delete.
	await page.getByRole('button', { name: 'More actions for web-renamed' }).click();
	await page.getByRole('menuitem', { name: 'Delete profile' }).click();
	await page.getByRole('button', { name: 'Delete profile', exact: true }).click();
	await expect(page.getByText('The profile is deleted.')).toBeVisible();
	await expect(page.getByText('No profiles yet')).toBeVisible();
});
