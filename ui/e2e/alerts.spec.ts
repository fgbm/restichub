import { expect, test, type Page } from '@playwright/test';

// The Overview dashboard, the Alerts list and the notification settings (FR-MN-01, FR-MN-03,
// FR-MN-04).
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
		localStorage.setItem('restichub.mock.alerts', 'sample');
	});
}

test('the overview shows the plan state, the hosts and the open alerts', async ({ page }) => {
	await seed(page);
	await signIn(page);

	await expect(page.getByRole('heading', { level: 1, name: 'Overview' })).toBeVisible();
	await expect(page.locator('.stats').getByText('Hosts offline')).toBeVisible();
	const plans = page.getByRole('list', { name: 'Plans' });
	await expect(plans.getByText('nightly-web').first()).toBeVisible();
	await expect(plans.locator('.status', { hasText: 'At risk' })).toBeVisible();
	await expect(page.getByRole('heading', { name: 'Open alerts' })).toBeVisible();
	await expect(page.getByText('Host offline').first()).toBeVisible();

	await page.getByRole('link', { name: 'All alerts' }).click();
	await expect(page.getByRole('heading', { level: 1, name: 'Alerts' })).toBeVisible();
	await expect(page.getByText('mac-build-01').first()).toBeVisible();
	await page
		.getByRole('group', { name: 'States' })
		.getByRole('button', { name: 'Resolved' })
		.click();
	await expect(page.getByText('Backup overdue')).toBeVisible();
	await expect(page.getByText('mac-build-01')).toHaveCount(0);
});

test('an Admin adds an email channel and a webhook, tests one and deletes one', async ({
	page
}) => {
	await seed(page);
	await signIn(page);

	await page
		.getByRole('navigation', { name: 'Primary navigation' })
		.getByRole('link', { name: 'Settings', exact: true })
		.click();
	await page.getByRole('link', { name: 'Channels', exact: true }).click();
	await expect(
		page.getByRole('heading', { level: 1, name: 'Notification channels' })
	).toBeVisible();
	await expect(page.getByText('ops-mail')).toBeVisible();

	// An email channel.
	await page.getByRole('button', { name: 'Add channel' }).click();
	const dialog = page.getByRole('dialog', { name: 'Add a channel' });
	await expect(dialog).toBeVisible();
	await page.locator('#channel-name').fill('team-mail');
	await page.locator('#channel-recipients').fill('team@example.com');
	await dialog.getByRole('button', { name: 'Save' }).click();
	await expect(page.getByText('The channel is created.')).toBeVisible();
	await expect(page.getByText('team@example.com')).toBeVisible();

	// A webhook: the address check refuses a local address, then the signing secret shows once.
	await page.getByRole('button', { name: 'Add channel' }).click();
	await page.locator('#channel-name').fill('team-hook');
	await page.locator('#channel-kind').selectOption({ label: 'Webhook' });
	await page.locator('#channel-url').fill('https://127.0.0.1/hook');
	await page
		.getByRole('dialog', { name: 'Add a channel' })
		.getByRole('button', { name: 'Save' })
		.click();
	await expect(page.getByText(/local or private/)).toBeVisible();
	await page.locator('#channel-url').fill('https://hooks.example.com/in/token');
	await page
		.getByRole('dialog', { name: 'Add a channel' })
		.getByRole('button', { name: 'Save' })
		.click();
	const secret = page.getByRole('dialog', { name: 'Signing secret' });
	await expect(secret).toBeVisible();
	await secret.getByRole('button', { name: 'Done' }).click();
	await expect(secret).toBeHidden();
	await expect(page.getByText('whsec_mock_9f3a1c7d5b2e4a60')).toHaveCount(0);

	// The test button reports the result.
	const hook = page.locator('li.item').filter({ hasText: 'team-hook' });
	await hook.getByRole('button', { name: 'Send test' }).click();
	await expect(page.getByText('The test message is sent.')).toBeVisible();

	// Delete a channel from its form: the item keeps two actions.
	await hook.getByRole('button', { name: 'team-hook' }).click();
	await page
		.getByRole('dialog', { name: 'Edit the channel' })
		.getByRole('button', { name: 'Delete channel' })
		.click();
	await page
		.getByRole('dialog', { name: 'Delete the channel?' })
		.getByRole('button', { name: 'Delete channel' })
		.click();
	await expect(page.getByText('The channel is deleted.')).toBeVisible();
});

test('an Admin switches an alert rule and saves it', async ({ page }) => {
	await signIn(page);
	await page
		.getByRole('navigation', { name: 'Primary navigation' })
		.getByRole('link', { name: 'Settings', exact: true })
		.click();
	await page.getByRole('link', { name: 'Alert rules', exact: true }).click();
	await expect(page.getByRole('heading', { level: 1, name: 'Alert rules' })).toBeVisible();
	await page.locator('#rule-job_warnings').click();
	await page.locator('#rule-agent_offline-minutes').fill('45');
	await page.getByRole('button', { name: 'Save rules' }).click();
	await expect(page.getByText('The rules are saved.')).toBeVisible();
	await expect(page.locator('#rule-agent_offline-minutes')).toHaveValue('45');
});
