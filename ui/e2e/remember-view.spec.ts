import { expect, test, type Page } from '@playwright/test';

// The UI keeps the view that a person chooses (FR-UI-05). A reload drops the mock session, so the
// tests move with the navigation of the shell. The unit tests cover the reload of the storage.
async function signIn(page: Page): Promise<void> {
	await page.addInitScript(() => {
		localStorage.setItem('restichub.mock.agents', 'sample');
		localStorage.setItem('restichub.mock.profiles', 'sample');
		localStorage.setItem('restichub.mock.plans', 'sample');
		localStorage.setItem('restichub.mock.repositories', 'sample');
	});
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.getByRole('button', { name: 'Sign in', exact: true }).click();
	await expect(page.getByRole('navigation', { name: 'Primary navigation' })).toBeVisible();
}

async function open(page: Page, name: string, heading = name): Promise<void> {
	await page
		.getByRole('navigation', { name: 'Primary navigation' })
		.getByRole('link', { name, exact: true })
		.click();
	await expect(page.getByRole('heading', { level: 1, name: heading })).toBeVisible();
}

// The shell routes a clicked link on the client, so the tests add a link to the page.
async function followLink(page: Page, href: string): Promise<void> {
	await page.evaluate((target) => {
		const link = document.createElement('a');
		link.id = 'probe-link';
		link.href = target;
		link.textContent = 'probe';
		document.body.append(link);
	}, href);
	await page.locator('#probe-link').click();
	await page.locator('#probe-link').evaluate((node) => node.remove());
}

test('ac_ui_04 ac_ui_05 ac_ui_06 the Jobs list keeps the view and a link wins', async ({
	page
}) => {
	await signIn(page);
	await open(page, 'Jobs');
	// The sample holds too few jobs for the page size control, so a link sets the size.
	await followLink(page, '/jobs?size=50');
	await page
		.getByRole('group', { name: 'States' })
		.getByRole('button', { name: 'Succeeded' })
		.click();
	await page.getByRole('button', { name: 'Host', exact: true }).click();
	await page.getByRole('checkbox').first().check();
	await page.keyboard.press('Escape');
	await expect(page).toHaveURL(/agent_id=/);
	await page.getByRole('button', { name: 'Started', exact: true }).click();
	await page.getByRole('menuitemradio', { name: 'Duration' }).click();
	await expect(page).toHaveURL(/size=50/);

	await open(page, 'Hosts');
	await open(page, 'Jobs');
	await expect(page).toHaveURL(/group=succeeded/);
	await expect(page).toHaveURL(/agent_id=/);
	await expect(page).toHaveURL(/size=50/);
	await expect(page).toHaveURL(/sort=duration/);
	await expect(
		page.getByRole('group', { name: 'States' }).getByRole('button', { name: 'Succeeded' })
	).toHaveAttribute('aria-pressed', 'true');

	// A link with parameters wins and does not change the kept view.
	const kept = await page.evaluate(() =>
		localStorage.getItem('restichub.v1.t_01.u_01.list./(app)/jobs')
	);
	await open(page, 'Hosts');
	await followLink(page, '/jobs?group=failed');
	await expect(page).toHaveURL(/group=failed/);
	await expect(page).not.toHaveURL(/size=50/);
	await expect(
		page.getByRole('group', { name: 'States' }).getByRole('button', { name: 'Failed' })
	).toHaveAttribute('aria-pressed', 'true');
	expect(
		await page.evaluate(() => localStorage.getItem('restichub.v1.t_01.u_01.list./(app)/jobs'))
	).toBe(kept);

	// "Reset" clears the kept filters; the page size stays.
	await open(page, 'Hosts');
	await open(page, 'Jobs');
	await expect(page).toHaveURL(/group=succeeded/);
	await page.getByRole('button', { name: 'Reset', exact: true }).click();
	await open(page, 'Hosts');
	await open(page, 'Jobs');
	await expect(page).not.toHaveURL(/group=|agent_id=/);
	await expect(page).toHaveURL(/size=50/);
});

test('ac_ui_09 the Restore page keeps the repository', async ({ page }) => {
	await signIn(page);
	await open(page, 'Restore');
	const select = page.locator('#restore-repository');
	// The page chose the first repository by itself: that is a kept choice too.
	await expect(select).toHaveValue('r_1');
	expect(await page.evaluate(() => JSON.stringify({ ...localStorage }))).toContain(
		'restore.repository'
	);
	await select.selectOption('r_2');
	await open(page, 'Jobs');
	await open(page, 'Restore');
	await expect(select).toHaveValue('r_2');
});

test('ac_ui_10 a section of the profile editor stays closed in the next record', async ({
	page
}) => {
	await signIn(page);
	await open(page, 'Profiles');
	await page.getByRole('button', { name: 'Add profile' }).click();
	await expect(page.getByRole('heading', { level: 1, name: 'New profile' })).toBeVisible();
	const paths = page.locator('#profile-paths-all');
	await expect(paths).toBeVisible();
	await page.getByRole('button', { name: 'Collapse all' }).click();
	await expect(paths).toBeHidden();

	await open(page, 'Hosts');
	await open(page, 'Profiles');
	await page.getByRole('button', { name: 'Add profile' }).click();
	await expect(page.getByRole('button', { name: 'Expand all' })).toBeVisible();
	await expect(page.locator('#profile-paths-all')).toBeHidden();
});
