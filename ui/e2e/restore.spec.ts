import { expect, test, type Page } from '@playwright/test';
import { pickRecord } from './pickers';

// The Restore screen lists the snapshots of a repository, shows the tree of a snapshot with lazy
// pages and starts a restore on a host (FR-RS-01 to FR-RS-04, UC-RS-01).
async function openRestore(page: Page): Promise<void> {
	await page.addInitScript(() => {
		localStorage.setItem('restichub.mock.agents', 'sample');
		localStorage.setItem('restichub.mock.repositories', 'sample');
	});
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.getByRole('button', { name: 'Sign in', exact: true }).click();
	await page
		.getByRole('navigation', { name: 'Primary navigation' })
		.getByRole('link', { name: 'Restore', exact: true })
		.click();
	await expect(page.getByRole('heading', { level: 1, name: 'Restore' })).toBeVisible();
}

test('the snapshot list takes filters and the tree loads a large directory by pages', async ({
	page
}) => {
	await openRestore(page);

	// The first visit reads the snapshots at once.
	await expect(page.getByRole('link', { name: /^Open snapshot / })).toHaveCount(8);
	await page.getByRole('button', { name: 'Host', exact: true }).click();
	await page.getByLabel('ws-anna').check();
	await page.keyboard.press('Escape');
	await expect(page.getByRole('link', { name: /^Open snapshot / })).toHaveCount(2);

	await page.getByRole('link', { name: 'Open snapshot a1b2d4e5' }).click();
	const tree = page.getByRole('tree', { name: 'Files of the snapshot' });
	await expect(tree.getByText('home', { exact: true })).toBeVisible();
	await tree.getByRole('button', { name: 'Expand home' }).click();
	await tree.getByRole('button', { name: 'Expand anna' }).click();
	// A link shows its target and has no expand button.
	await expect(tree.getByText('→ /srv/www')).toBeVisible();
	await expect(tree.getByRole('button', { name: 'Expand current-project' })).toHaveCount(0);

	// The breadcrumb link to the list keeps its filters.
	await page
		.getByRole('navigation', { name: 'Breadcrumb' })
		.getByRole('link', { name: 'Restore' })
		.click();
	await expect(page.getByText('Host: ws-anna')).toBeVisible();
	await expect(page.getByRole('link', { name: /^Open snapshot / })).toHaveCount(2);

	// A directory with 2500 entries comes in pages of 1000.
	await page.getByRole('button', { name: 'Reset', exact: true }).click();
	await expect(page.getByRole('link', { name: /^Open snapshot / })).toHaveCount(8);
	await page.getByRole('link', { name: 'Open snapshot a1b2c3d4' }).click();
	await tree.getByRole('button', { name: 'Expand srv' }).click();
	await tree.getByRole('button', { name: 'Expand www' }).click();
	await tree.getByRole('button', { name: 'Expand uploads' }).click();
	await expect(tree.getByText('upload-00000.dat')).toBeVisible();
	await expect(tree.getByText('upload-01000.dat')).toHaveCount(0);
	await tree.getByRole('button', { name: 'Load more' }).click();
	await expect(tree.getByText('upload-01000.dat')).toBeVisible({ timeout: 20_000 });
	await tree.getByRole('button', { name: 'Load more' }).click();
	await expect(tree.getByText('upload-02499.dat')).toBeVisible({ timeout: 20_000 });
	await expect(tree.getByRole('button', { name: 'Load more' })).toHaveCount(0);
});

test('a restore asks for the target, warns about a system path and opens the job', async ({
	page
}) => {
	await openRestore(page);
	await page
		.getByRole('link', { name: /^Open snapshot / })
		.first()
		.click();
	const tree = page.getByRole('tree', { name: 'Files of the snapshot' });
	await tree.getByRole('checkbox', { name: 'Select etc' }).check();
	await expect(page.getByText('Selected: 1')).toBeVisible();

	await page.getByRole('button', { name: 'Restore…' }).click();
	const dialog = page.getByRole('dialog');
	await expect(dialog.getByRole('region', { name: 'Summary' })).toContainText('Selected paths: 1');
	await pickRecord(page, dialog.locator('#restore-agent'), 'srv-db-01');
	// The target starts as a folder in the restore root that the host reports.
	await expect(dialog.getByLabel('Target path')).toHaveValue(
		/^\/var\/lib\/restichub-restore\/[0-9a-f]{8}$/
	);
	await dialog.getByLabel('Target path').fill('/etc');
	// One message for a path outside the roots; the system warning stays away.
	await expect(dialog.getByText(/This path is outside the restore roots/)).toBeVisible();
	await expect(dialog.getByText('System path', { exact: true })).toHaveCount(0);
	await dialog.getByLabel('Target path').fill('relative');
	await expect(dialog.getByText('Enter an absolute path.')).toBeVisible();
	await expect(dialog.getByRole('button', { name: 'Start restore' })).toBeDisabled();
	// An empty target does not disable the action silently: the attempt names the field.
	await dialog.getByLabel('Target path').fill('');
	await dialog.getByRole('button', { name: 'Start restore' }).click();
	await expect(dialog.getByText('This field is required.')).toBeVisible();
	await expect(dialog.getByLabel('Target path')).toBeFocused();
	await dialog.getByLabel('Target path').fill('/var/lib/restichub-restore/job-1');
	await expect(dialog.getByText('System path', { exact: true })).toHaveCount(0);
	await dialog.getByLabel('Existing files').selectOption({ label: 'Replace a file that differs' });
	await dialog.getByLabel('Dry run').check();
	await expect(dialog.getByLabel('Verify after the restore')).toBeDisabled();
	await dialog.getByRole('button', { name: 'Start restore' }).click();

	// The job detail shows the restore with its summary.
	await expect(page.getByRole('heading', { level: 1, name: /^j_/ })).toBeVisible();
	await expect(page.getByText('Files restored', { exact: true })).toBeVisible({ timeout: 10_000 });
	// The job page lists what the operator chose.
	await expect(page.getByText('Restore parameters')).toBeVisible();
	await expect(page.getByText('/var/lib/restichub-restore/job-1')).toBeVisible();
	await expect(page.getByText('Replace a file that differs')).toBeVisible();
});
