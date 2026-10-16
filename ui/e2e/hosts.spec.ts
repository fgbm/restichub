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

test('a created registration appears pending and a cancel removes it', async ({ page }) => {
	await signIn(page);
	await page.getByRole('link', { name: 'Hosts' }).click();
	await expect(page.getByRole('heading', { level: 1, name: 'Hosts' })).toBeVisible();

	await page.getByRole('button', { name: 'Add host' }).click();
	await page.locator('#host-labels').fill('web');
	await page.getByRole('button', { name: 'Create token' }).click();
	await page.locator('.copy-field').first().waitFor();
	await page.getByRole('button', { name: 'Done' }).click();

	await expect(page.getByText('Pending registrations')).toBeVisible();
	const pending = page.getByRole('list', { name: 'Pending registrations' });
	await expect(pending.getByText('labels: web')).toBeVisible();

	// The closed add dialog stays mounted for a frame and shows a Cancel button too.
	await pending.getByRole('button', { name: /^Cancel the token/ }).click();
	const dialog = page.getByRole('dialog', { name: 'Cancel the registration?' });
	await dialog.getByRole('button', { name: 'Cancel' }).click();

	await expect(page.getByText('Pending registrations')).toBeHidden();
});

test('an Admin renames a host', async ({ page }) => {
	await page.addInitScript(() => localStorage.setItem('restichub.mock.agents', 'sample'));
	await signIn(page);
	await page
		.getByRole('navigation', { name: 'Primary navigation' })
		.getByRole('link', { name: 'Hosts', exact: true })
		.click();
	await page.getByText('win-fs01', { exact: true }).waitFor();

	await page.getByRole('button', { name: 'More actions for win-fs01', exact: true }).click();
	await page.getByRole('menuitem', { name: 'Rename' }).click();

	const dialog = page.getByRole('dialog', { name: 'Rename the host' });
	await dialog.locator('#host-rename').fill('win-fs01-renamed');
	await dialog.getByRole('button', { name: 'Rename' }).click();

	await expect(page.getByText('win-fs01-renamed', { exact: true })).toBeVisible();
});

test('the hosts list fits 1440 px without a horizontal scroll (FR-ID-11)', async ({ page }) => {
	await page.setViewportSize({ width: 1440, height: 900 });
	await page.addInitScript(() => localStorage.setItem('restichub.mock.agents', 'sample'));
	await signIn(page);
	await page
		.getByRole('navigation', { name: 'Primary navigation' })
		.getByRole('link', { name: 'Hosts', exact: true })
		.click();
	await page.getByText('win-fs01', { exact: true }).waitFor();

	const overflow = await page.evaluate(
		() => document.documentElement.scrollWidth - document.documentElement.clientWidth
	);
	expect(overflow).toBeLessThanOrEqual(0);
	await expect(page.getByText('Possible duplicate').first()).toBeVisible();
});

test('a host page shows the host and its dangerous actions open small dialogs', async ({
	page
}) => {
	await page.addInitScript(() => localStorage.setItem('restichub.mock.agents', 'sample'));
	await signIn(page);
	await page
		.getByRole('navigation', { name: 'Primary navigation' })
		.getByRole('link', { name: 'Hosts', exact: true })
		.click();
	await page.getByRole('link', { name: 'srv-db-01', exact: true }).click();

	await expect(page.getByRole('heading', { level: 1, name: 'srv-db-01' })).toBeVisible();
	await expect(page.locator('[data-section="restore"] input[readonly]')).toHaveValue(
		'/var/lib/restichub-restore'
	);
	await expect(page.getByRole('link', { name: 'Restore to this host' })).toBeVisible();

	await page.getByRole('button', { name: 'Revoke agent' }).click();
	const dialog = page.getByRole('dialog', { name: 'Revoke the host?' });
	await expect(dialog).toBeVisible();
	await dialog.getByRole('button', { name: 'Cancel' }).click();
	await expect(dialog).toBeHidden();

	await page.getByRole('button', { name: 'Rename' }).click();
	const rename = page.getByRole('dialog', { name: 'Rename the host' });
	await rename.locator('#host-rename').fill('srv-db-02');
	await rename.getByRole('button', { name: 'Rename' }).click();
	await expect(page.getByRole('heading', { level: 1, name: 'srv-db-02' })).toBeVisible();
});

test('the hosts list keeps the row menu inside a 390 px screen', async ({ page }) => {
	await page.setViewportSize({ width: 390, height: 844 });
	await page.addInitScript(() => localStorage.setItem('restichub.mock.agents', 'sample'));
	await signIn(page);
	await page
		.getByRole('navigation', { name: 'Primary navigation' })
		.getByRole('link', { name: 'Hosts', exact: true })
		.click();
	await page.getByText('win-fs01', { exact: true }).waitFor();

	const button = page.getByRole('button', { name: 'More actions for win-fs01', exact: true });
	await expect(button).toBeVisible();
	const box = await button.boundingBox();
	expect(box).not.toBeNull();
	expect((box?.x ?? 0) + (box?.width ?? 0)).toBeLessThanOrEqual(390);
	const overflow = await page.evaluate(
		() => document.documentElement.scrollWidth - document.documentElement.clientWidth
	);
	expect(overflow).toBeLessThanOrEqual(0);
	// A narrow list keeps the first two fields of line 2.
	const item = page.locator('li.item').filter({ hasText: 'win-fs01' }).first();
	await expect(item.locator('.item__field--extra').first()).toBeHidden();
	await expect(item.locator('.item__field').first()).toBeVisible();
});
