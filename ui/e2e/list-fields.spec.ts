import { expect, test, type Page } from '@playwright/test';

// The list items show the facts that the server adds: the recovery point state of a plan, the
// health of a repository, the old certificate of a host and the counts of the job tabs (FR-UI-01).
async function signIn(page: Page): Promise<void> {
	await page.addInitScript(() => {
		for (const key of ['agents', 'profiles', 'plans', 'repositories', 'scripts']) {
			localStorage.setItem(`restichub.mock.${key}`, 'sample');
		}
	});
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.getByRole('button', { name: 'Sign in', exact: true }).click();
	await expect(page.getByRole('navigation', { name: 'Primary navigation' })).toBeVisible();
}

async function open(page: Page, section: string): Promise<void> {
	await page
		.getByRole('navigation', { name: 'Primary navigation' })
		.getByRole('link', { name: section, exact: true })
		.click();
	await expect(page.getByRole('heading', { level: 1, name: section })).toBeVisible();
}

test('fr_ui_01 a plan item shows its recovery point state and its last backup', async ({
	page
}) => {
	await signIn(page);
	await open(page, 'Plans');
	const list = page.getByRole('list', { name: 'Plans' });
	const late = list.locator('li.item').filter({ hasText: 'nightly-db' });
	await expect(late.locator('.status', { hasText: 'At risk' })).toBeVisible();
	await expect(late.getByText('backup 22 h ago')).toBeVisible();
	await expect(list.getByText('backup 7 h ago')).toBeVisible();
	await page
		.getByRole('group', { name: 'States' })
		.getByRole('button', { name: 'Need attention 1' })
		.click();
	await expect(list.locator('li.item')).toHaveCount(1);
});

test('fr_ui_01 a repository item shows the lock, the last check and the snapshots', async ({
	page
}) => {
	await signIn(page);
	await open(page, 'Repositories');
	const list = page.getByRole('list', { name: 'Repositories' });
	const local = list.locator('li.item').filter({ hasText: 'nas-local' });
	await expect(local.locator('.status', { hasText: 'Stale lock' })).toBeVisible();
	await expect(local.getByText('checked 6 d ago')).toBeVisible();
	await expect(list.getByText('146 snapshots')).toBeVisible();
});

test('fr_ui_01 a host item shows the use of an old certificate in the danger tone', async ({
	page
}) => {
	await signIn(page);
	await open(page, 'Hosts');
	const item = page.locator('li.item').filter({ hasText: 'win-dc01' });
	await expect(item.locator('.status', { hasText: 'Old certificate used' })).toBeVisible();
	await expect(item.locator('.item__lead')).toHaveClass(/tone--danger/);
});

test('fr_ui_01 the job tabs show the server counts and a tab narrows the list', async ({
	page
}) => {
	await signIn(page);
	await open(page, 'Jobs');
	const tabs = page.getByRole('group', { name: 'States' });
	const warnings = tabs.getByRole('button', { name: /^Warnings \d+$/ });
	await expect(warnings).toBeVisible();
	await expect(tabs.getByRole('button', { name: /^Failed \d+$/ })).toBeVisible();
	await warnings.click();
	const list = page.getByRole('list', { name: 'Jobs' });
	await expect(list.locator('.status[title="Succeeded with warnings"]').first()).toBeVisible();
	await expect(list.locator('.status', { hasText: 'Running' })).toHaveCount(0);
	await expect(page).toHaveURL(/group=succeeded_with_warnings/);
});
