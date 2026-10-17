import { expect, test, type Page } from '@playwright/test';
import { pickRecord } from './pickers';

// The Plans screen lists the plans, creates one on the plan page and deletes one (FR-SC-01,
// FR-SC-07).
async function signIn(page: Page): Promise<void> {
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.getByRole('button', { name: 'Sign in', exact: true }).click();
	await expect(page.getByRole('navigation', { name: 'Primary navigation' })).toBeVisible();
}

// A plan needs a profile and a host. The mock starts empty, so the test asks for the samples.
async function seed(page: Page): Promise<void> {
	await page.addInitScript(() => {
		localStorage.setItem('restichub.mock.agents', 'sample');
		localStorage.setItem('restichub.mock.profiles', 'sample');
		localStorage.setItem('restichub.mock.repositories', 'sample');
	});
}

test('the plans screen creates and deletes a plan', async ({ page }) => {
	await seed(page);
	await signIn(page);

	await page
		.getByRole('navigation', { name: 'Primary navigation' })
		.getByRole('link', { name: 'Plans', exact: true })
		.click();
	await expect(page.getByRole('heading', { level: 1, name: 'Plans' })).toBeVisible();
	await expect(page).toHaveTitle(`Plans · ResticHub`);
	await expect(page.getByText('No plans yet')).toBeVisible();

	await page.getByRole('button', { name: 'Add plan' }).click();
	await expect(page).toHaveURL(/\/plans\/new$/);
	await expect(page.getByRole('heading', { level: 1, name: 'New plan' })).toBeVisible();
	await expect(page).toHaveTitle(`New plan · ResticHub`);
	await page.locator('#plan-name').fill('nightly-web');
	// The profile field shows where each profile writes and finds a profile by its repository.
	await page.locator('#plan-profile').click();
	await page.getByRole('combobox', { name: 'Find: Profile' }).fill('offsite');
	const option = page.getByRole('option', { name: 'web-daily' });
	await expect(option).toContainText('offsite-s3');
	await option.click();
	await expect(page.locator('#plan-profile')).toContainText('web-daily · offsite-s3');
	await pickRecord(page, page.locator('#plan-agent'), 'win-fs01');
	await page.locator('#plan-cron').fill('0 2 * * *');
	await page.locator('#plan-time-zone').fill('Europe/Berlin');
	await page.getByRole('button', { name: 'Save' }).click();

	await expect(page.getByText('The plan is created.')).toBeVisible();
	await expect(page.getByRole('heading', { level: 1, name: 'Plans' })).toBeVisible();
	await expect(page.getByText('nightly-web')).toBeVisible();
	// Line 2 shows the schedule as a phrase; the cron expression is in its tooltip.
	await expect(page.getByText('every day at 02:00')).toHaveAttribute(
		'title',
		'0 2 * * * · Europe/Berlin'
	);

	// Delete the plan on its page: the item keeps two actions.
	await page.getByRole('link', { name: 'nightly-web' }).click();
	await expect(page.getByRole('heading', { level: 1, name: 'nightly-web' })).toBeVisible();
	await page.getByRole('button', { name: 'Delete plan' }).click();
	await page.getByRole('dialog').getByRole('button', { name: 'Delete plan', exact: true }).click();
	await expect(page.getByText('The plan is deleted.')).toBeVisible();
	await expect(page.getByText('No plans yet')).toBeVisible();
});

test('the plan page shows the next runs of an existing plan', async ({ page }) => {
	await seed(page);
	await page.addInitScript(() => localStorage.setItem('restichub.mock.plans', 'sample'));
	await signIn(page);
	await page
		.getByRole('navigation', { name: 'Primary navigation' })
		.getByRole('link', { name: 'Plans', exact: true })
		.click();
	// The Overview lists the sample plan too, so wait for the Plans screen first.
	await expect(page.getByRole('heading', { level: 1, name: 'Plans' })).toBeVisible();

	// The plan name opens the plan page.
	await page.getByRole('link', { name: 'nightly-web', exact: true }).click();
	await expect(page).toHaveURL(/\/plans\/[^/]+$/);
	await expect(page.getByRole('heading', { level: 1, name: 'nightly-web' })).toBeVisible();
	await expect(page.getByRole('navigation', { name: 'Breadcrumb' })).toContainText('Plans');
	// The schedule section holds the next five runs of the plan.
	await expect(page.getByText('Next 5 runs')).toBeVisible();
	await expect(page.locator('.runs li')).toHaveCount(5);
});
