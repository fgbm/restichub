import { expect, test, type Page } from '@playwright/test';

// The Profiles screen lists the profiles of the organization. The editor page creates and edits
// one, the list deletes one and shows the compiled steps of the preview (FR-PF-01, FR-PF-04,
// FR-PF-05).
async function signIn(page: Page): Promise<void> {
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.getByRole('button', { name: 'Sign in', exact: true }).click();
	await expect(page.getByRole('navigation', { name: 'Primary navigation' })).toBeVisible();
}

// A profile needs a repository. The mock holds no repository, so the test creates one first.
async function createRepository(page: Page): Promise<void> {
	await page
		.getByRole('navigation', { name: 'Primary navigation' })
		.getByRole('link', { name: 'Repositories', exact: true })
		.click();
	await page.getByRole('button', { name: 'Add repository' }).click();
	await page.locator('#repo-name').fill('s3-prod');
	await page.locator('#repo-endpoint').fill('https://s3.example.net');
	await page.locator('#repo-bucket').fill('backups-prod');
	await page.locator('#repo-password').fill('repo-password');
	await page.locator('#repo-s3-access-key').fill('access-key');
	await page.locator('#repo-s3-secret-key').fill('secret-key');
	await page.getByRole('button', { name: 'Save', exact: true }).click();
	await expect(page.getByText('The repository is created.')).toBeVisible();
}

async function openProfiles(page: Page): Promise<void> {
	await page
		.getByRole('navigation', { name: 'Primary navigation' })
		.getByRole('link', { name: 'Profiles', exact: true })
		.click();
}

const breadcrumb = (page: Page) => page.getByRole('navigation', { name: 'Breadcrumb' });

test('the profiles screen creates, previews, edits and deletes a profile', async ({ page }) => {
	// One host reports its facts, so the preview has a target.
	await page.addInitScript(() => localStorage.setItem('restichub.mock.agents', 'sample'));
	await signIn(page);
	await createRepository(page);

	await openProfiles(page);
	await expect(page.getByRole('heading', { level: 1, name: 'Profiles' })).toBeVisible();
	await expect(page).toHaveTitle('Profiles · ResticHub');
	await expect(page.getByText('No profiles yet')).toBeVisible();

	// Create: the editor is a page, and the top bar names it.
	await page.getByRole('button', { name: 'Add profile' }).click();
	await expect(page).toHaveURL(/\/profiles\/new$/);
	await expect(page.getByRole('heading', { level: 1, name: 'New profile' })).toBeVisible();
	await expect(breadcrumb(page)).toContainText('Profiles');
	await expect(breadcrumb(page)).toContainText('New profile');
	await page.locator('#profile-name').fill('web-daily');
	await page.locator('#profile-paths-all').fill('/srv');
	await page.locator('#profile-paths-linux').fill('/etc');
	await page.getByRole('switch', { name: 'Turn on the section "Tags"' }).click();
	await page.locator('#profile-tags').fill('daily, web ');
	await expect(page.getByRole('button', { name: 'Remove the tag daily' })).toBeVisible();
	await expect(page.getByRole('button', { name: 'Remove the tag web' })).toBeVisible();
	// The side panel shows the command of the profile as it is typed.
	await expect(page.getByText('--tag daily')).toBeVisible();
	await page.getByRole('button', { name: 'Save' }).click();

	await expect(page.getByText('The profile is created.')).toBeVisible();
	await expect(page).toHaveURL(/\/profiles$/);
	await expect(page.getByRole('link', { name: 'web-daily' })).toBeVisible();

	// Edit: the name link opens the page, the name changes.
	await page.getByRole('link', { name: 'web-daily' }).click();
	await expect(page.getByRole('heading', { level: 1, name: 'web-daily' })).toBeVisible();

	// The preview of the profile page shows the steps and the command line with masked secrets.
	await page.getByRole('button', { name: 'Preview' }).click();
	const preview = page.getByRole('dialog', { name: 'Preview' });
	await expect(preview).toBeVisible();
	await page.locator('#preview-host').selectOption({ label: 'win-fs01' });
	await expect(preview.getByText('restic backup', { exact: false })).toBeVisible();
	await expect(preview.getByText('***', { exact: false }).first()).toBeVisible();
	await page.keyboard.press('Escape');

	await expect(breadcrumb(page)).toContainText('web-daily');
	await page.locator('#profile-name').fill('web-renamed');
	await page.getByRole('button', { name: 'Save' }).click();
	await expect(page.getByText('The profile is updated.')).toBeVisible();
	await expect(page.getByRole('link', { name: 'web-renamed' })).toBeVisible();

	// Delete.
	await page
		.getByRole('list', { name: 'Profiles' })
		.locator('li.item')
		.filter({ hasText: 'web-renamed' })
		.getByRole('button', { name: 'Delete profile' })
		.click();
	await page
		.getByRole('dialog')
		.getByRole('button', { name: 'Delete profile', exact: true })
		.click();
	await expect(page.getByText('The profile is deleted.')).toBeVisible();
	await expect(page.getByText('No profiles yet')).toBeVisible();
});

test('the profile name is the only link of the item and a copy opens a new profile', async ({
	page
}) => {
	await page.addInitScript(() => localStorage.setItem('restichub.mock.profiles', 'sample'));
	await signIn(page);
	await createRepository(page);
	await openProfiles(page);
	await expect(page.getByRole('link', { name: 'web-daily' })).toBeVisible();

	// The remaining part of the item is plain text (UI principles 2.3).
	const item = page.getByRole('list', { name: 'Profiles' }).locator('li.item').first();
	await expect(item.getByRole('link')).toHaveCount(1);
	await item.locator('.item__meta').click();
	await expect(page).toHaveURL(/\/profiles$/);

	await item.getByRole('button', { name: 'Make a copy' }).click();
	await expect(page).toHaveURL(/\/profiles\/new\?from=p_1$/);
	await expect(page.locator('#profile-name')).toHaveValue('web-daily (copy)');
	await page.getByRole('button', { name: 'Cancel' }).click();
	await expect(page).toHaveURL(/\/profiles$/);
	await page.getByRole('link', { name: 'web-daily' }).click();
	await expect(page).toHaveURL(/\/profiles\/p_1$/);
	await expect(page.locator('#profile-name')).toHaveValue('web-daily');
});

test('editing a profile keeps the retention fields', async ({ page }) => {
	// The sample profile holds a keep-yearly rule and a snapshot filter.
	await page.addInitScript(() => localStorage.setItem('restichub.mock.profiles', 'sample'));
	await signIn(page);
	await createRepository(page);
	await openProfiles(page);
	await page.getByRole('link', { name: 'web-daily' }).click();

	// A collapsed section shows its summary. The retention section opens on a click.
	await page.getByRole('button', { name: /^Retention/ }).click();
	await expect(page.locator('#profile-keep-yearly')).toHaveValue('2');
	await page.getByText('Limit the rules to some snapshots').click();
	await expect(page.locator('#profile-filter-hosts')).toHaveValue('web-*');
	await page.locator('#profile-name').fill('web-renamed');
	await page.getByRole('button', { name: 'Save' }).click();
	await expect(page.getByText('The profile is updated.')).toBeVisible();

	// A second edit keeps the fields after the rename.
	await page.getByRole('link', { name: 'web-renamed' }).click();
	await page.getByRole('button', { name: /^Retention/ }).click();
	await expect(page.locator('#profile-keep-yearly')).toHaveValue('2');
	await page.getByText('Limit the rules to some snapshots').click();
	await expect(page.locator('#profile-filter-hosts')).toHaveValue('web-*');
});

test('the limits section creates and edits a profile with the job limits', async ({ page }) => {
	await signIn(page);
	await createRepository(page);
	await openProfiles(page);
	await page.getByRole('button', { name: 'Add profile' }).click();

	await page.locator('#profile-name').fill('limits-demo');
	await page.locator('#profile-paths-all').fill('/srv');
	await page.getByRole('button', { name: /^Run limits/ }).click();
	// The fields start with the defaults of the server.
	await expect(page.locator('#profile-stall-timeout')).toHaveValue('60');
	await expect(page.locator('#profile-max-duration')).toHaveValue('48');
	await page.locator('#profile-stall-timeout').fill('90');
	await page.locator('#profile-max-duration').fill('12');
	await page.getByRole('button', { name: 'Save' }).click();
	await expect(page.getByText('The profile is created.')).toBeVisible();

	await page.getByRole('link', { name: 'limits-demo' }).click();
	await page.getByRole('button', { name: /^Run limits/ }).click();
	await expect(page.locator('#profile-stall-timeout')).toHaveValue('90');
	await expect(page.locator('#profile-max-duration')).toHaveValue('12');

	// An empty field is omitted, so the stored value stays.
	await page.locator('#profile-stall-timeout').fill('');
	await page.locator('#profile-max-duration').fill('5');
	await page.getByRole('button', { name: 'Save' }).click();
	await expect(page.getByText('The profile is updated.')).toBeVisible();
	await page.getByRole('link', { name: 'limits-demo' }).click();
	await page.getByRole('button', { name: /^Run limits/ }).click();
	await expect(page.locator('#profile-stall-timeout')).toHaveValue('90');
	await expect(page.locator('#profile-max-duration')).toHaveValue('5');
});
