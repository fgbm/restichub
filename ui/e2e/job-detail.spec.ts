import { expect, test, type Page } from '@playwright/test';

// The job detail screen shows the progress, the steps, the summary and the log of one job
// (FR-MN-02, FR-LG-01).
async function signIn(page: Page): Promise<void> {
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.getByRole('button', { name: 'Sign in', exact: true }).click();
	await expect(page.getByRole('navigation', { name: 'Primary navigation' })).toBeVisible();
}

async function openJobs(page: Page): Promise<void> {
	await page.addInitScript(() => {
		localStorage.setItem('restichub.mock.agents', 'sample');
		localStorage.setItem('restichub.mock.profiles', 'sample');
		localStorage.setItem('restichub.mock.plans', 'sample');
	});
	await signIn(page);
	await page
		.getByRole('navigation', { name: 'Primary navigation' })
		.getByRole('link', { name: 'Jobs', exact: true })
		.click();
	await expect(page.getByRole('heading', { level: 1, name: 'Jobs' })).toBeVisible();
}

test('a finished job shows its summary, its steps and its log', async ({ page }) => {
	await openJobs(page);
	// The sample holds a succeeded job and a job with warnings; the first row of a status is enough.
	await page.locator('#jobs-status').selectOption({ label: 'Succeeded' });
	await page
		.getByRole('link', { name: /Open the job nightly-web/ })
		.first()
		.click();

	await expect(page.getByRole('heading', { level: 1, name: 'nightly-web' })).toBeVisible();
	const summary = page.locator('section').filter({ hasText: 'Statistics from the restic summary' });
	await expect(summary.getByText('1.4 GiB', { exact: true })).toBeVisible();
	await expect(summary.getByText('a1b2c3d4')).toBeVisible();
	await expect(page.getByText('3m 12s').first()).toBeVisible();

	await expect(page.getByText('2 reported')).toBeVisible();
	const log = page.getByRole('log', { name: 'Job log' });
	await expect(log.getByText('snapshot a1b2c3d4 saved')).toBeVisible();
	await expect(log.getByText('INFO').first()).toBeVisible();
});

test('a running job shows its progress and the log follows the new lines', async ({ page }) => {
	await openJobs(page);
	await page.locator('#jobs-status').selectOption({ label: 'Running' });
	await page.getByRole('link', { name: /Open the job nightly-web/ }).click();

	await expect(page.getByRole('progressbar', { name: 'Progress' })).toBeVisible();
	await expect(page.getByText('Live log')).toBeVisible();
	await expect(page.getByText('following')).toBeVisible();
	const lines = page.getByRole('log', { name: 'Job log' }).locator('.log__line');
	const before = await lines.count();
	// The mock adds a line every four seconds, and the screen polls every two seconds.
	await expect.poll(async () => lines.count(), { timeout: 12_000 }).toBeGreaterThan(before);

	// A person who scrolls up stops the following.
	await page.getByRole('button', { name: 'Pause' }).click();
	await expect(page.getByRole('button', { name: 'Follow' })).toBeVisible();
});
