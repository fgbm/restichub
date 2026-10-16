import { expect, test, type Page } from '@playwright/test';

// The Repositories screen lists the repositories of the organization. A repository is created and
// edited on its own page and deleted in a dialog (FR-RP-01, FR-RP-05).
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
	await expect(page).toHaveURL(/\/repositories\/new$/);
	await expect(page.getByRole('heading', { level: 1, name: 'New repository' })).toBeVisible();
	await page.locator('#repo-name').fill('s3-prod');
	await page.locator('#repo-endpoint').fill('https://s3.example.net');
	await page.locator('#repo-bucket').fill('backups-prod');
	await page.locator('#repo-password').fill('repo-password');
	await page.locator('#repo-s3-access-key').fill('access-key');
	await page.locator('#repo-s3-secret-key').fill('secret-key');
	await page.getByRole('button', { name: 'Save', exact: true }).click();

	await expect(page.getByText('The repository is created.')).toBeVisible();
	await expect(page.getByText('s3-prod')).toBeVisible();
	await expect(page.getByText('Not initialized')).toBeVisible();
	await expect(page.getByText('s3://backups-prod')).toBeVisible();

	// Edit: the name changes and the secret fields stay empty.
	await page.getByRole('button', { name: 'More actions for s3-prod' }).click();
	await page.getByRole('menuitem', { name: 'Edit' }).click();
	await expect(page.getByRole('heading', { level: 1, name: 'Edit repository' })).toBeVisible();
	// The password field is create-only: the password cannot be replaced until FR-RP-06.
	await expect(page.locator('#repo-password')).toHaveCount(0);
	await expect(page.getByText('cannot be replaced')).toBeVisible();
	await page.locator('#repo-name').fill('s3-prod-renamed');
	await page.getByRole('button', { name: 'Save', exact: true }).click();
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
	await page.getByRole('button', { name: 'Save', exact: true }).click();
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

// The repository maintenance: the warning for a missing host, the schedules in the dialog and the
// run-now action (FR-RP-08, FR-SC-09).
async function addLocalRepository(page: Page, name: string, host: string | null): Promise<void> {
	await page.getByRole('button', { name: 'Add repository' }).click();
	await page.locator('#repo-name').fill(name);
	await page.locator('#repo-kind').selectOption('local');
	await page.locator('#repo-path').fill(`/srv/backup/${name}`);
	await page.locator('#repo-password').fill('repo-password');
	if (host) await page.locator('#repo-agent').selectOption({ label: host });
	await page.getByRole('button', { name: 'Save', exact: true }).click();
	await expect(page.getByText(name, { exact: true })).toBeVisible();
}

test('the repository maintenance shows the warning, edits the schedules and runs a task', async ({
	page
}) => {
	await page.addInitScript(() => localStorage.setItem('restichub.mock.agents', 'sample'));
	await signIn(page);
	await page
		.getByRole('navigation', { name: 'Primary navigation' })
		.getByRole('link', { name: 'Repositories', exact: true })
		.click();

	// A new repository starts with the maintenance section in the time zone of the browser.
	await page.getByRole('button', { name: 'Add repository' }).click();
	const zone = await page.evaluate(() => Intl.DateTimeFormat().resolvedOptions().timeZone);
	await expect(page.locator('#maintenance-time-zone')).toHaveValue(zone);
	await page.getByRole('button', { name: 'Cancel', exact: true }).click();

	await addLocalRepository(page, 'repo-m', 'nas-backup');
	await addLocalRepository(page, 'repo-nohost', null);
	// A repository without a maintenance host shows a warning in the list.
	await expect(page.getByText('No maintenance host')).toBeVisible();

	// Run now needs an initialized repository.
	await page.getByRole('button', { name: 'More actions for repo-m' }).click();
	await expect(page.getByRole('menuitem', { name: 'Run retention now' })).toHaveCount(0);
	await page.getByRole('menuitem', { name: 'Initialize' }).click();
	const job = page.getByRole('dialog', { name: 'Initialize the repository?' });
	await job.getByRole('button', { name: 'Start' }).click();
	await expect(job.getByText('The repository is ready.')).toBeVisible({ timeout: 10_000 });
	await page.keyboard.press('Escape');

	await page.getByRole('button', { name: 'More actions for repo-m' }).click();
	await page.getByRole('menuitem', { name: 'Run retention now' }).click();
	await expect(page.getByText('The maintenance job is queued.')).toBeVisible();

	// Unlock all asks for the repository name in a centered danger dialog.
	await page.getByRole('button', { name: 'More actions for repo-m' }).click();
	await page.getByRole('menuitem', { name: 'Unlock all' }).click();
	const unlock = page.getByRole('dialog', { name: 'Remove all locks?' });
	await expect(unlock.getByText('This also removes the locks of running operations')).toBeVisible();
	const unlockButton = unlock.getByRole('button', { name: 'Unlock all' });
	await expect(unlockButton).toBeDisabled();
	await unlock.getByLabel('Type the repository name to confirm').fill('repo-m');
	await expect(unlockButton).toBeEnabled();
	await unlockButton.click();
	await expect(page.getByText('The unlock job is queued.')).toBeVisible();

	// Edit the schedules: a preset fills the cron, the preview lists the next runs.
	await page.getByRole('button', { name: 'More actions for repo-m' }).click();
	await page.getByRole('menuitem', { name: 'Edit' }).click();
	await expect(page.getByRole('heading', { level: 1, name: 'Edit repository' })).toBeVisible();
	await expect(page.locator('#maintenance-retention-cron')).toHaveValue('0 3 * * 0');
	await page.getByRole('button', { name: 'Daily at 03:00' }).click();
	await expect(page.locator('#maintenance-retention-cron')).toHaveValue('0 3 * * *');
	await expect(page.locator('.runs li').first()).toBeVisible();
	await page.getByRole('button', { name: 'Save', exact: true }).click();
	await expect(page.getByText('The repository is updated.')).toBeVisible();

	await page.getByRole('button', { name: 'More actions for repo-m' }).click();
	await page.getByRole('menuitem', { name: 'Edit' }).click();
	await expect(page.locator('#maintenance-retention-cron')).toHaveValue('0 3 * * *');
});
