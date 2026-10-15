import { expect, test, type Page } from '@playwright/test';

// The Plans screen lists the plans, creates one from the dialog and deletes one (FR-SC-01,
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
	const dialog = page.getByRole('dialog', { name: 'Add a plan' });
	await expect(dialog).toBeVisible();
	await page.locator('#plan-name').fill('nightly-web');
	await page.locator('#plan-profile').selectOption({ label: 'web-daily' });
	await page.locator('#plan-agent').selectOption({ label: 'win-fs01' });
	await page.locator('#plan-cron').fill('0 2 * * *');
	await page.locator('#plan-time-zone').fill('Europe/Berlin');
	await dialog.getByRole('button', { name: 'Save' }).click();

	await expect(page.getByText('The plan is created.')).toBeVisible();
	await expect(page.getByText('nightly-web')).toBeVisible();
	// The schedule column shows the phrase and the cron expression.
	await expect(page.getByText('Every day at 02:00')).toBeVisible();

	// Delete the plan.
	await page.getByRole('button', { name: 'More actions for nightly-web' }).click();
	await page.getByRole('menuitem', { name: 'Delete plan' }).click();
	await page.getByRole('button', { name: 'Delete plan', exact: true }).click();
	await expect(page.getByText('The plan is deleted.')).toBeVisible();
	await expect(page.getByText('No plans yet')).toBeVisible();
});

test('the plan dialog shows the next runs of an existing plan', async ({ page }) => {
	await seed(page);
	await page.addInitScript(() => localStorage.setItem('restichub.mock.plans', 'sample'));
	await signIn(page);
	await page
		.getByRole('navigation', { name: 'Primary navigation' })
		.getByRole('link', { name: 'Plans', exact: true })
		.click();
	// The Overview lists the sample plan too, so wait for the Plans screen first.
	await expect(page.getByRole('heading', { level: 1, name: 'Plans' })).toBeVisible();
	await expect(page.getByText('nightly-web')).toBeVisible();

	await page.getByRole('button', { name: 'More actions for nightly-web' }).click();
	await page.getByRole('menuitem', { name: 'Edit' }).click();
	const dialog = page.getByRole('dialog', { name: 'Edit the plan' });
	await expect(dialog).toBeVisible();
	// The dialog holds the next five runs of the plan.
	await expect(dialog.getByText('Next 5 runs')).toBeVisible();
	const runs = dialog.locator('.runs li');
	await expect(runs).toHaveCount(5);
});
