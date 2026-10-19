import { expect, test, type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';

// Captures the screens 1 to 5 of the variables design for the owner in Russian, at 1440 px and, for
// the list at the caret and the tags, at 360 px. It runs only with CAPTURE and CAPTURE_DIR set, so
// CI skips it (FR-PF-03, FR-PF-10).
const dir = process.env.CAPTURE_DIR;
test.skip(!process.env.CAPTURE || !dir, 'set CAPTURE and CAPTURE_DIR to write the screenshots');
test.describe.configure({ mode: 'serial' });

const out = dir as string;

const nav = (page: Page) => page.getByRole('navigation', { name: 'Основная навигация' });

async function start(page: Page, width: number): Promise<void> {
	mkdirSync(out, { recursive: true });
	// The open list is 420 px high, so the narrow frame is high enough to show the whole panel.
	await page.setViewportSize({ width, height: 1000 });
	await page.emulateMedia({ colorScheme: 'light' });
	await page.addInitScript(() => {
		for (const key of ['agents', 'profiles', 'plans', 'scripts', 'repositories']) {
			localStorage.setItem(`restichub.mock.${key}`, 'sample');
		}
		localStorage.setItem('restichub.locale', 'ru');
	});
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.getByRole('button', { name: 'Войти', exact: true }).click();
	await expect(nav(page)).toBeVisible();
	await nav(page).getByRole('link', { name: 'Профили', exact: true }).click();
	await page.getByRole('link', { name: 'db-nightly' }).click();
	await page.getByRole('button', { name: 'Развернуть всё' }).click();
}

/** Replaces the value of a field and puts the caret at the given index, then types the text. */
async function typeIn(page: Page, selector: string, value: string, caret: number, text: string) {
	const field = page.locator(selector);
	await field.click();
	await field.evaluate(
		(node, [next, at]) => {
			const element = node as HTMLTextAreaElement;
			element.value = next as string;
			element.setSelectionRange(at as number, at as number);
			element.dispatchEvent(new Event('input', { bubbles: true }));
		},
		[value, caret]
	);
	if (text !== '') {
		await page.keyboard.type(text);
		await expect(page.getByRole('listbox', { name: 'Переменные' })).toBeVisible();
	}
}

async function shot(page: Page, name: string): Promise<void> {
	await page.waitForTimeout(200);
	await page.screenshot({ path: `${out}/${name}.png` });
}

for (const width of [1440, 360]) {
	test(`screen 1: the list at the caret line (${width} px)`, async ({ page }) => {
		await start(page, width);
		await typeIn(page, '#profile-paths-all', '/var/backups/\n/etc/pg', 13, '{{');
		await page.locator('[data-section="paths"]').scrollIntoViewIfNeeded();
		await shot(page, `screen-1-paths-${width}`);
	});

	test(`screen 3: the list below the tags field (${width} px)`, async ({ page }) => {
		await start(page, width);
		const tags = page.locator('#profile-tags');
		await tags.click();
		await page.keyboard.type('db-{{');
		await expect(page.getByRole('listbox', { name: 'Переменные' })).toBeVisible();
		await page.locator('[data-section="tags"]').scrollIntoViewIfNeeded();
		await shot(page, `screen-3-tags-${width}`);
		await page.keyboard.press('Enter');
		await page.keyboard.press('Enter');
		await expect(page.getByRole('button', { name: 'Убрать db-{{param.pg_dir}}' })).toBeVisible();
		await shot(page, `screen-3-tag-added-${width}`);
	});
}

test('screen 2: the filter and the disabled secret', async ({ page }) => {
	await start(page, 1440);
	await typeIn(page, '#profile-paths-all', '/var/log/', 10, '{{pg');
	await page.locator('[data-section="paths"]').scrollIntoViewIfNeeded();
	await shot(page, 'screen-2-filter-secret-1440');
});

test('screen 2: the keys of the host labels', async ({ page }) => {
	await start(page, 1440);
	await typeIn(page, '#profile-paths-all', '/srv/', 5, '{{agent.label.');
	await page.locator('[data-section="paths"]').scrollIntoViewIfNeeded();
	await shot(page, 'screen-2-label-keys-1440');
});

test('screen 4: no match and the button Declare', async ({ page }) => {
	await start(page, 1440);
	await typeIn(page, '#profile-paths-all', '/srv/', 5, '{{zz');
	await page.locator('[data-section="paths"]').scrollIntoViewIfNeeded();
	await shot(page, 'screen-4-declare-1440');
});

test('screen 4: the hint under the field and the name rule', async ({ page }) => {
	await start(page, 1440);
	await typeIn(page, '#profile-paths-all', '/var/log/{{param.pg_old}}', 25, '');
	await page.locator('[data-section="paths"]').scrollIntoViewIfNeeded();
	await shot(page, 'screen-4-hint-1440');

	// A name that the server refuses keeps the hint and loses the button.
	await typeIn(page, '#profile-paths-all', '/var/log/{{param.Pg}}', 21, '');
	await shot(page, 'screen-4-hint-invalid-1440');
});

test('screen 5: the inserted variable and the count of uses', async ({ page }) => {
	await start(page, 1440);
	await typeIn(page, '#profile-paths-all', '/var/backups//daily', 13, '{{pg');
	await page.keyboard.press('Enter');
	await expect(page.locator('#profile-paths-all')).toHaveValue(
		'/var/backups/{{param.pg_dir}}/daily'
	);
	await page.locator('[data-section="paths"]').scrollIntoViewIfNeeded();
	await shot(page, 'screen-5-inserted-1440');

	await page.locator('[data-section="parameters"]').scrollIntoViewIfNeeded();
	await shot(page, 'screen-5-parameters-1440');
});
