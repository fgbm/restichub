import { expect, test } from '@playwright/test';

// Runs against a real control plane in multi mode (scripts/ui-smoke.sh), not the mock service
// (issue #26). The setup token comes from the start log of the control plane.
const setupToken = process.env.SMOKE_SETUP_TOKEN ?? '';
const username = 'smoke-admin';
const password = 'a long smoke password';
const organization = `smoke-${Date.now().toString(36)}`;

test.skip(!setupToken, 'SMOKE_SETUP_TOKEN is not set: no real control plane is running.');

test('setup, sign in, create an organization and find its audit record', async ({ page }) => {
	await page.goto('/setup');
	await page.locator('#setup-token').fill(setupToken);
	await page.locator('#setup-username').fill(username);
	await page.locator('#setup-display-name').fill('Smoke Admin');
	await page.locator('#setup-password').fill(password);
	await page.locator('#setup-confirm').fill(password);
	await page.getByRole('button', { name: 'Create the Platform Admin' }).click();

	await expect(page).toHaveURL(/\/sign-in/);
	await page.locator('#signin-username').fill(username);
	await page.locator('#signin-password').fill(password);
	await page.getByRole('button', { name: 'Sign in', exact: true }).click();
	await expect(page.getByRole('navigation', { name: 'Primary navigation' })).toBeVisible();

	await page.goto('/platform/tenants');
	await page.getByRole('button', { name: 'Create organization' }).click();
	await page.locator('#tenant-name').fill(organization);
	await page
		.getByRole('dialog')
		.getByRole('button', { name: 'Create organization', exact: true })
		.click();
	await expect(page.getByRole('button', { name: organization, exact: true })).toBeVisible();

	await page.goto('/platform/audit');
	const record = page.locator('li.item').filter({ hasText: 'Create organization' }).first();
	await expect(record).toBeVisible();
	// The details panel of the record names the object.
	await record.locator('.item__title').click();
	await expect(page.getByRole('dialog')).toContainText(/tenant [0-9a-f-]{36}/);

	// A member of the new organization opens the event stream of the shell once (FR-UI-02).
	const csrf = { 'X-ResticHub-CSRF': '1' };
	const tenants = await (await page.request.get('/api/v1/platform/tenants')).json();
	const tenantId = tenants.items.find((item: { name: string }) => item.name === organization).id;
	const created = await page.request.post('/api/v1/platform/users', {
		headers: csrf,
		data: {
			username: 'smoke-member',
			display_name: 'Smoke Member',
			password,
			memberships: [{ tenant_id: tenantId, role: 'admin' }]
		}
	});
	expect(created.status()).toBe(201);
	await page.request.post('/api/v1/auth/sign-out', { headers: csrf });

	const stream = page.waitForResponse(
		(response) => new URL(response.url()).pathname === '/api/v1/events'
	);
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('smoke-member');
	await page.locator('#signin-password').fill(password);
	await page.getByRole('button', { name: 'Sign in', exact: true }).click();
	const response = await stream;
	expect(response.status()).toBe(200);
	expect(response.headers()['content-type']).toContain('text/event-stream');
	expect(response.headers()['x-accel-buffering']).toBe('no');
});
