import { expect, test, type Page } from '@playwright/test';
import { chooseTab } from './filters';

// Server scripts: the Scripts section of the profile form, the script templates, the trust mode
// of the hosts and the log of each step of a job (FR-HK-08 to FR-HK-12, ADR-0037).
async function signIn(page: Page): Promise<void> {
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.getByRole('button', { name: 'Sign in', exact: true }).click();
	await expect(page.getByRole('navigation', { name: 'Primary navigation' })).toBeVisible();
}

async function openSample(page: Page, link: string): Promise<void> {
	await page.addInitScript(() => {
		localStorage.setItem('restichub.mock.agents', 'sample');
		localStorage.setItem('restichub.mock.profiles', 'sample');
		localStorage.setItem('restichub.mock.plans', 'sample');
		localStorage.setItem('restichub.mock.scripts', 'sample');
		localStorage.setItem('restichub.mock.repositories', 'sample');
	});
	await signIn(page);
	await page
		.getByRole('navigation', { name: 'Primary navigation' })
		.getByRole('link', { name: link, exact: true })
		.click();
}

test('a profile keeps its scripts in two ordered lists and saves a reorder', async ({ page }) => {
	await openSample(page, 'Profiles');
	await page.getByRole('link', { name: 'db-nightly', exact: true }).click();
	await page.getByRole('button', { name: /^Scripts on the host/ }).click();

	const before = page.getByTestId('scripts-before');
	const after = page.getByTestId('scripts-after');
	await expect(before.getByTestId('script-card')).toHaveCount(2);
	await expect(after.getByTestId('script-card')).toHaveCount(1);
	const titles = before.locator('.card__title');
	await expect(titles).toHaveText(['Check free disk space', 'Dump the PostgreSQL database']);

	// The second script moves up, so the dump runs first.
	await before.getByTestId('script-card').nth(1).getByRole('button', { name: 'Move up' }).click();
	await expect(titles).toHaveText(['Dump the PostgreSQL database', 'Check free disk space']);
	await page.getByRole('button', { name: 'Save' }).click();
	await expect(page.getByText('The profile is updated.')).toBeVisible();

	// The saved order comes back when the page opens again.
	await page.getByRole('link', { name: 'db-nightly' }).click();
	await page.getByRole('button', { name: /^Scripts on the host/ }).click();
	await expect(page.getByTestId('scripts-before').locator('.card__title')).toHaveText([
		'Dump the PostgreSQL database',
		'Check free disk space'
	]);
});

test('a script is added from a template and the host without the trust mode is named', async ({
	page
}) => {
	await openSample(page, 'Profiles');
	// The host of this profile is win-fs01, which did not turn on the trust mode.
	await page.getByRole('link', { name: 'web-daily', exact: true }).click();
	await page.getByRole('button', { name: /^Scripts on the host/ }).click();
	await expect(page.getByTestId('scripts-before').getByTestId('script-card')).toHaveCount(0);

	await page
		.getByTestId('scripts-before')
		.getByRole('button', { name: 'From a script template' })
		.click();
	const picker = page.getByRole('dialog', { name: 'Add a script from a template' });
	await picker.getByLabel('Search templates').fill('stop');
	await picker.getByTestId('template-choices').getByRole('button', { name: 'Use' }).first().click();
	const card = page.getByTestId('scripts-before').getByTestId('script-card');
	await expect(card).toHaveCount(1);
	await expect(card.locator('.card__title')).toHaveText('Stop a service (Linux)');
	await expect(card.getByLabel('Variable')).toHaveValue('SERVICE_NAME');

	await expect(page.getByText('A host does not accept scripts')).toBeVisible();
	await expect(
		page.getByText('These hosts did not turn on the trust mode: win-fs01.')
	).toBeVisible();
	await expect(page.locator('input[readonly]')).toHaveValue(
		'restichub-agent.exe trust-server-scripts --enable'
	);
});

test('the script templates screen opens a built-in template read-only and copies it', async ({
	page
}) => {
	await openSample(page, 'Settings');
	await page.getByRole('link', { name: 'Script templates' }).click();
	await expect(page.getByRole('heading', { level: 1, name: 'Script templates' })).toBeVisible();
	// The own templates open first; the built-in ones are on their tab.
	await expect(page.getByText('Remove old dumps')).toBeVisible();
	await chooseTab(page, 'Built-in');
	await expect(page.getByText('PostgreSQL dump with pg_dump (Linux)')).toBeVisible();

	await page
		.locator('li.item')
		.filter({ hasText: 'PostgreSQL dump with pg_dump (Linux)' })
		.getByRole('button', { name: 'Make a copy' })
		.click();
	const dialog = page.getByRole('dialog', { name: 'Add a script template' });
	await expect(dialog.getByLabel(/^Template/)).toHaveValue(
		'PostgreSQL dump with pg_dump (Linux) copy'
	);
	await dialog.getByLabel(/^Template/).fill('Database dump');
	await dialog.getByRole('button', { name: 'Save' }).click();
	await expect(page.getByText('The template is created.')).toBeVisible();

	// A built-in template cannot be deleted.
	await expect(
		page
			.locator('li.item')
			.filter({ hasText: 'PostgreSQL dump with pg_dump (Linux)' })
			.getByRole('button', { name: 'Delete' })
	).toHaveCount(0);

	await chooseTab(page, 'Own');
	await expect(page.getByText('Database dump', { exact: true })).toBeVisible();
	await page
		.locator('li.item')
		.filter({ hasText: 'Database dump' })
		.getByRole('button', { name: 'Delete' })
		.click();
	await page
		.getByRole('dialog', { name: 'Delete the template?' })
		.getByRole('button', { name: 'Delete' })
		.click();
	await expect(page.getByText('The template is deleted.')).toBeVisible();
});

test('the host page shows the trust mode and the command', async ({ page }) => {
	await openSample(page, 'Hosts');
	await page.getByRole('link', { name: 'win-fs01', exact: true }).click();
	await expect(page.getByRole('heading', { level: 1, name: 'win-fs01' })).toBeVisible();
	await expect(page.locator('#scripts input[readonly]')).toHaveValue(
		'restichub-agent.exe trust-server-scripts --enable'
	);
	// The breadcrumb is the way back (UI principles 1.1).
	await page
		.getByRole('navigation', { name: 'Breadcrumb' })
		.getByRole('link', { name: 'Hosts' })
		.click();
	await page.getByRole('link', { name: 'srv-db-01', exact: true }).click();
	await expect(page.locator('#scripts').getByText('bash, sh')).toBeVisible();
});

test('a finished job lists each script as a step with its own log', async ({ page }) => {
	await openSample(page, 'Jobs');
	await page
		.getByRole('list', { name: 'Jobs' })
		.getByRole('link', { name: 'nightly-db', exact: true })
		.first()
		.click();
	await expect(page.getByText('4 of 4 complete')).toBeVisible();

	const dump = page.getByRole('button', { name: 'Show the log of Dump the PostgreSQL database' });
	await expect(dump).toContainText('Script');
	await expect(dump).toContainText('Before');
	await dump.click();
	const log = page.getByTestId('step-log-script-1');
	await expect(log.getByText('dump written: /var/backups/postgres/app.dump')).toBeVisible();
	// The log of another step stays out of this section.
	await expect(log.getByText('snapshot a1b2c3d4 saved')).toHaveCount(0);

	// The backup step has its own section too.
	await page.getByRole('button', { name: 'Show the log of Backup' }).click();
	await expect(
		page.getByTestId('step-log-backup').getByText('snapshot a1b2c3d4 saved')
	).toBeVisible();
});
