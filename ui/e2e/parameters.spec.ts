import { expect, test, type Page } from '@playwright/test';

// The parameter of a profile: the host stores a value, the plan reads it, and the plan saves with
// its own overrides (FR-PF-09 to FR-PF-12, AC-PF-06, AC-PF-08, ADR-0047). The mock holds the
// sample profile db-nightly with four parameters and the host srv-db-01.
async function signIn(page: Page): Promise<void> {
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.getByRole('button', { name: 'Sign in', exact: true }).click();
	await expect(page.getByRole('navigation', { name: 'Primary navigation' })).toBeVisible();
}

async function sample(page: Page): Promise<void> {
	await page.addInitScript(() => {
		for (const key of ['agents', 'profiles', 'plans', 'scripts', 'repositories']) {
			localStorage.setItem(`restichub.mock.${key}`, 'sample');
		}
	});
}

const nav = (page: Page) => page.getByRole('navigation', { name: 'Primary navigation' });

test('ac_pf_06_a_host_value_reaches_the_plan_and_the_plan_saves', async ({ page }) => {
	await sample(page);
	await signIn(page);

	// The profile list shows the count of the parameters as a chip of line 2.
	await nav(page).getByRole('link', { name: 'Profiles', exact: true }).click();
	const item = page
		.locator('.item')
		.filter({ has: page.getByRole('link', { name: 'db-nightly' }) });
	await expect(item).toContainText('4 parameters');

	// The host holds the value. The operator changes it in the table of the host page.
	await nav(page).getByRole('link', { name: 'Hosts', exact: true }).click();
	await page.getByRole('link', { name: 'srv-db-01' }).click();
	const value = page.getByLabel('Value of pg_dir');
	await expect(value).toHaveValue('/var/backups/pg');
	await value.fill('/srv/pg');
	await page.getByRole('button', { name: 'Save the values' }).click();
	await expect(page.getByText('The values are saved.')).toBeVisible();

	// The plan reads the new value from the host and shows its source.
	await nav(page).getByRole('link', { name: 'Plans', exact: true }).click();
	await page.getByRole('link', { name: 'nightly-db' }).click();
	const section = page.locator('[data-section="parameters"]');
	await expect(section.getByText('/srv/pg')).toBeVisible();
	await expect(section.getByRole('cell', { name: 'host', exact: true }).first()).toBeVisible();

	// The plan saves with its override and keeps the value of the host.
	await page.locator('#plan-cron').fill('0 3 * * *');
	await page.getByRole('button', { name: 'Save', exact: true }).click();
	await expect(page.getByRole('heading', { level: 1, name: 'Plans' })).toBeVisible();
	await page.getByRole('link', { name: 'nightly-db' }).click();
	await expect(page.locator('[data-section="parameters"]').getByText('/srv/pg')).toBeVisible();
});

test('ac_pf_07_a_new_required_parameter_names_the_plans_that_it_blocks', async ({ page }) => {
	await sample(page);
	await signIn(page);

	// A required parameter without a value on the host leaves the plan of the profile without one.
	await nav(page).getByRole('link', { name: 'Profiles', exact: true }).click();
	await page.getByRole('link', { name: 'db-nightly' }).click();
	const section = page.getByRole('button', { name: /^Parameters/ });
	if ((await section.getAttribute('aria-expanded')) !== 'true') await section.click();
	await page.getByRole('button', { name: 'Add parameter' }).click();
	await page.getByLabel('Parameter', { exact: true }).last().fill('pg_port');
	await page.getByLabel('Required', { exact: true }).last().check();
	await page.getByRole('button', { name: 'Save', exact: true }).click();

	// The form shows the refusal with the plans and keeps the save enabled (screen 12).
	await expect(
		page.getByText(
			'The profile is not saved: plans would stay without the required parameter pg_port.'
		)
	).toBeVisible();
	await expect(page.getByRole('link', { name: 'nightly-db' })).toBeVisible();
	await expect(page.getByRole('button', { name: 'Save', exact: true })).toBeEnabled();
});

test('the closed pickers of a plan keep the label and the parts on one row', async ({ page }) => {
	await sample(page);
	await signIn(page);

	await nav(page).getByRole('link', { name: 'Plans', exact: true }).click();
	await page.getByRole('button', { name: 'Add plan' }).click();
	await expect(page.getByRole('heading', { level: 1, name: 'New plan' })).toBeVisible();

	// The parts of the detail are a chip and a text, and both stay on the line of the label.
	for (const id of ['#plan-profile', '#plan-repository', '#plan-agent']) {
		const row = await page.locator(id).evaluate((field) => {
			const value = field.querySelector('.rh-record__value') as HTMLElement;
			const line = field.querySelector('.record-line') as HTMLElement;
			const a = value.getBoundingClientRect();
			const b = line.getBoundingClientRect();
			return {
				directChild: line.parentElement === field,
				centres: Math.abs(a.top + a.height / 2 - (b.top + b.height / 2))
			};
		});
		expect(row.directChild, `${id} keeps the parts in the row of the field`).toBe(true);
		expect(row.centres, `${id} puts the parts on the line of the label`).toBeLessThan(1.5);
	}
});

for (const width of [1440, 1280]) {
	test(`the kind of a parameter shows its longest option at ${width} px`, async ({ page }) => {
		await page.setViewportSize({ width, height: 900 });
		await sample(page);
		await signIn(page);

		await nav(page).getByRole('link', { name: 'Profiles', exact: true }).click();
		await page.getByRole('link', { name: 'db-nightly' }).click();
		const section = page.getByRole('button', { name: /^Parameters/ });
		if ((await section.getAttribute('aria-expanded')) !== 'true') await section.click();

		// The cell keeps the width of the longest option, "Secret" or "Секрет", so no letter is cut.
		const kind = page.locator('[data-section="parameters"] select').first();
		await expect(kind).toBeVisible();
		const sizes = await kind.evaluate((select) => {
			const style = getComputedStyle(select);
			const context = document.createElement('canvas').getContext('2d') as CanvasRenderingContext2D;
			context.font = `${style.fontWeight} ${style.fontSize} ${style.fontFamily}`;
			const longest = Math.max(
				...[...select.options].map((option) => context.measureText(option.text).width)
			);
			const room =
				select.getBoundingClientRect().width -
				parseFloat(style.paddingLeft) -
				parseFloat(style.paddingRight) -
				parseFloat(style.borderLeftWidth) -
				parseFloat(style.borderRightWidth);
			return { longest, room };
		});
		expect(sizes.room).toBeGreaterThanOrEqual(sizes.longest);
	});
}
