import { expect, test, type Page } from '@playwright/test';

// The Jobs screen lists the jobs, cancels a queued one and shows a manual run (FR-SC-09).
async function signIn(page: Page): Promise<void> {
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.getByRole('button', { name: 'Sign in', exact: true }).click();
	await expect(page.getByRole('navigation', { name: 'Primary navigation' })).toBeVisible();
}

async function seed(page: Page): Promise<void> {
	await page.addInitScript(() => {
		localStorage.setItem('restichub.mock.agents', 'sample');
		localStorage.setItem('restichub.mock.profiles', 'sample');
		localStorage.setItem('restichub.mock.plans', 'sample');
	});
}

async function openJobs(page: Page): Promise<void> {
	await page
		.getByRole('navigation', { name: 'Primary navigation' })
		.getByRole('link', { name: 'Jobs', exact: true })
		.click();
	await expect(page.getByRole('heading', { level: 1, name: 'Jobs' })).toBeVisible();
	await expect(page).toHaveTitle('Jobs · ResticHub');
}

test('the jobs screen lists the jobs and cancels a queued job', async ({ page }) => {
	await seed(page);
	await signIn(page);
	await openJobs(page);

	const table = page.getByRole('table');
	await expect(table.getByText('Running')).toBeVisible();
	await expect(table.getByText('Succeeded', { exact: true })).toBeVisible();
	await expect(table.getByText('Succeeded with warnings')).toBeVisible();
	await expect(table.getByText('The profile no longer fits the host.')).toBeVisible();
	await expect(table.getByText('The repository stayed locked after every retry.')).toBeVisible();

	// Cancel the queued job after the confirmation dialog.
	const queued = table.getByRole('row').filter({ hasText: 'Waiting' });
	await queued.getByRole('button', { name: /More actions for the job/ }).click();
	await page.getByRole('menuitem', { name: 'Cancel job' }).click();
	const dialog = page.getByRole('dialog', { name: 'Cancel the job?' });
	await expect(dialog).toBeVisible();
	await dialog.getByRole('button', { name: 'Cancel job' }).click();
	await expect(page.getByText('The job is cancelled.')).toBeVisible();
	await expect(table.getByText('Cancelled')).toBeVisible();
	await expect(table.getByText('Waiting')).toHaveCount(0);
});

test('the status filter narrows the list', async ({ page }) => {
	await seed(page);
	await signIn(page);
	await openJobs(page);

	await page.locator('#jobs-status').selectOption({ label: 'Succeeded' });
	const table = page.getByRole('table');
	await expect(table.getByText('Succeeded', { exact: true })).toBeVisible();
	await expect(table.getByText('Running')).toHaveCount(0);
});

test('a manual run from the plans screen appears as a waiting job', async ({ page }) => {
	await seed(page);
	await signIn(page);

	await page
		.getByRole('navigation', { name: 'Primary navigation' })
		.getByRole('link', { name: 'Plans', exact: true })
		.click();
	await page.getByRole('button', { name: 'More actions for nightly-web' }).click();
	await page.getByRole('menuitem', { name: 'Run now' }).click();
	await expect(page.getByText('The job is queued. It starts when the host is free.')).toBeVisible();

	await openJobs(page);
	// The sample holds one waiting job; the manual run adds a second.
	await expect(page.getByRole('table').getByText('Waiting')).toHaveCount(2);
});
