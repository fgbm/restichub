import { expect, test, type Page } from '@playwright/test';
import { chooseTab } from './filters';

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
	await chooseTab(page, 'Succeeded');
	await page
		.getByRole('list', { name: 'Jobs' })
		.getByRole('link', { name: 'nightly-web', exact: true })
		.first()
		.click();

	// The top bar shows the way: "Jobs / <job id>".
	const crumbs = page.getByRole('navigation', { name: 'Breadcrumb' });
	await expect(crumbs.getByRole('link', { name: 'Jobs' })).toBeVisible();
	await expect(crumbs.locator('[aria-current="page"]')).toHaveText(/^j_/);
	await expect(page.getByRole('heading', { level: 1, name: /^j_/ })).toBeVisible();
	await expect(
		page.getByText(/Plan nightly-web · host .* · triggered by the schedule/)
	).toBeVisible();
	const summary = page.locator('section').filter({ hasText: 'Statistics from the restic summary' });
	await expect(summary.getByText('1.4 GiB', { exact: true })).toBeVisible();
	await expect(summary.getByText('a1b2c3d4')).toBeVisible();
	await expect(page.getByText('3m 12s').first()).toBeVisible();

	await expect(page.getByText('2 of 2 complete')).toBeVisible();
	await expect(page.getByRole('button', { name: 'Run again' })).toBeEnabled();
	await expect(page.getByRole('button', { name: 'Cancel job' })).toHaveCount(0);
	// One log per job, inside its step: the step is collapsed and shows the count of its lines.
	await expect(page.getByText('Live log')).toHaveCount(0);
	await expect(page.getByRole('log')).toHaveCount(0);
	const toggle = page.getByRole('button', { name: /Show the log of/ }).first();
	await expect(toggle).toContainText(/\d+ lines/);
	await toggle.click();
	const log = page.getByRole('log').first();
	await expect(log.getByText('snapshot a1b2c3d4 saved')).toBeVisible();
	await expect(log.getByText('INFO').first()).toBeVisible();

	await page.getByRole('combobox', { name: 'Level' }).first().selectOption('problems');
	await expect(log.getByText('snapshot a1b2c3d4 saved')).toHaveCount(0);
	await page.getByRole('combobox', { name: 'Level' }).first().selectOption('all');
	await page.getByRole('searchbox', { name: 'Search the log' }).first().fill('a1b2c3d4');
	await expect(log.getByText('snapshot a1b2c3d4 saved')).toBeVisible();

	const download = page.waitForEvent('download');
	await page.getByRole('button', { name: 'Download log' }).click();
	expect((await download).suggestedFilename()).toMatch(/^job-j_.*-log\.txt$/);
});

test('a running job shows its progress and the log follows the new lines', async ({ page }) => {
	await openJobs(page);
	await chooseTab(page, 'Running');
	await page
		.getByRole('list', { name: 'Jobs' })
		.getByRole('link', { name: 'nightly-web', exact: true })
		.first()
		.click();

	await expect(page.getByRole('progressbar', { name: 'Progress' })).toBeVisible();
	// The steps that did not start show with the count of the complete ones.
	await expect(page.getByText('0 of 3 complete')).toBeVisible();
	await expect(page.getByText('queued', { exact: true })).toHaveCount(2);
	await expect(page.getByText(/step 1 of 3/)).toBeVisible();
	await expect(page.getByRole('button', { name: 'Run again' })).toBeDisabled();

	// The cancel action asks in a dialog in the middle of the screen.
	await page.getByRole('button', { name: 'Cancel job' }).click();
	const dialog = page.getByRole('dialog', { name: 'Cancel the job?' });
	await expect(dialog).toBeVisible();
	const box = await dialog.boundingBox();
	const viewport = page.viewportSize();
	expect(Math.abs((box?.x ?? 0) + (box?.width ?? 0) / 2 - (viewport?.width ?? 0) / 2)).toBeLessThan(
		2
	);
	await dialog.getByRole('button', { name: 'Keep the job' }).click();
	await expect(dialog).toBeHidden();

	// The running step is open and followed; the separate live panel is gone.
	await expect(page.getByText('Live log')).toHaveCount(0);
	const follow = page.getByRole('button', { name: 'Follow' });
	await expect(follow).toHaveAttribute('aria-pressed', 'true');
	const lines = page.getByRole('log').first().locator('.step-log__line');
	const before = await lines.count();
	// The mock adds a line every four seconds, and the screen polls every two seconds.
	await expect.poll(async () => lines.count(), { timeout: 12_000 }).toBeGreaterThan(before);

	// A person who scrolls up stops the following; a click resumes it.
	await page
		.getByRole('log')
		.first()
		.evaluate((view) => (view.scrollTop = 0));
	await expect(follow).toHaveAttribute('aria-pressed', 'false');
	await follow.click();
	await expect(follow).toHaveAttribute('aria-pressed', 'true');
});

test('a running job shows its limit and no acceptance window', async ({ page }) => {
	await openJobs(page);
	await chooseTab(page, 'Running');
	await page
		.getByRole('list', { name: 'Jobs' })
		.getByRole('link', { name: 'nightly-web', exact: true })
		.first()
		.click();

	// A run has no fixed time limit: the host stops a step that makes no progress (ADR-0039).
	await expect(page.getByText('Limit', { exact: true })).toBeVisible();
	await expect(page.getByText('Maximum: 48h')).toBeVisible();
	await expect(page.getByText('No progress: 1h')).toBeVisible();
	await expect(page.getByText('Expires', { exact: true })).toHaveCount(0);
	await expect(page.getByText('Accept until', { exact: true })).toHaveCount(0);

	// restic reports an unreadable file and goes on: the log shows it as a warning.
	const log = page.getByRole('log').first();
	await expect(log.getByText('Access to the cloud file is denied')).toBeVisible();
	await expect(log.getByText('WARN').first()).toBeVisible();
});
