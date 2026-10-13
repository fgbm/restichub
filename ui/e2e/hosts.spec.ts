import { expect, test, type Page } from '@playwright/test';

// The Hosts screen lists the agents and creates an enrollment token (FR-ID-01, ADR-0026).
async function signIn(page: Page): Promise<void> {
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.getByRole('button', { name: 'Sign in', exact: true }).click();
	await expect(page.getByRole('navigation', { name: 'Primary navigation' })).toBeVisible();
}

test('the hosts screen creates an enrollment token and shows it once', async ({ page }) => {
	await signIn(page);
	await page.getByRole('link', { name: 'Hosts' }).click();
	await expect(page.getByRole('heading', { level: 1, name: 'Hosts' })).toBeVisible();
	// The mock holds no agent, so the empty state shows.
	await expect(page.getByText('No hosts')).toBeVisible();

	await page.getByRole('button', { name: 'Add host' }).click();
	await page.locator('#host-labels').fill('web');
	await page.getByRole('button', { name: 'Create token' }).click();

	const origin = await page.evaluate(() => window.location.origin);
	const value = 're_EXAMPLEONLYVALUE00000000000000000000000000';
	const fields = page.locator('.copy-field input');
	await expect(fields.first()).toHaveValue(value);
	// The command carries this installation URL, not a placeholder (ADR-0032).
	await expect(fields.nth(1)).toHaveValue(
		`restichub-agent register --url ${origin} --token ${value}`
	);
	await expect(page.getByText('The token works once and expires in one hour.')).toBeVisible();
});
