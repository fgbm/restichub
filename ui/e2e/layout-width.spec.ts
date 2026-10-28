import { expect, test, type Page } from '@playwright/test';

// A section that shows a subnav keeps it on every page. Otherwise the content column moves when a
// person opens a record that belongs to the list (design/style/ui-principles.md, 1.3). The column
// keeps its place also when the window scrollbar appears, because it is centred on the window and
// not on the space that the scrollbar leaves.

const WIDTHS = [1440, 1280, 1024];

interface Place {
	subnavX: number;
	contentX: number;
	contentWidth: number;
}

async function signIn(page: Page): Promise<void> {
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.getByRole('button', { name: 'Sign in', exact: true }).click();
	await expect(page.getByRole('navigation', { name: 'Primary navigation' })).toBeVisible();
}

// A reload drops the mock session, so the test follows the links of the shell.
async function openList(page: Page, path: string): Promise<void> {
	await page.locator('a[href="/settings"]').first().click();
	await page.locator(`a[href="${path}"]`).first().click();
	await expect(page).toHaveURL(new RegExp(`${path}(\\?.*)?$`));
	await expect(page.locator('li.item').first()).toBeVisible();
}

async function openRecord(page: Page, path: string): Promise<void> {
	await page.locator('li.item a.item__title').first().click();
	await expect(page).toHaveURL(new RegExp(`${path}/[^/?]+`));
	// A record page draws its sections after its load, and the list is gone by then.
	await expect(page.locator('.record').first()).toBeVisible();
}

async function place(page: Page): Promise<Place> {
	for (const selector of ['.subnav', '.settings__content']) {
		expect(await page.locator(selector).count(), `the settings page has no ${selector}`).toBe(1);
	}
	const subnav = await page.locator('.subnav').boundingBox();
	const content = await page.locator('.settings__content').boundingBox();
	expect(subnav, 'the settings subnav is not on the page').not.toBeNull();
	expect(content, 'the settings content column is not on the page').not.toBeNull();
	return {
		subnavX: Math.round(subnav?.x ?? 0),
		contentX: Math.round(content?.x ?? 0),
		contentWidth: Math.round(content?.width ?? 0)
	};
}

async function overflows(page: Page): Promise<boolean> {
	return page.evaluate(() => {
		const root = document.scrollingElement as Element;
		return root.scrollHeight > root.clientHeight;
	});
}

function expectSamePlace(list: Place, record: Place, where: string): void {
	expect(Math.abs(record.subnavX - list.subnavX), `${where}: the subnav moved`).toBeLessThanOrEqual(
		1
	);
	expect(
		Math.abs(record.contentX - list.contentX),
		`${where}: the content moved`
	).toBeLessThanOrEqual(1);
}

function expectSameWidth(list: Place, record: Place, where: string): void {
	expect(
		Math.abs(record.contentWidth - list.contentWidth),
		`${where}: the content changed its width`
	).toBeLessThanOrEqual(1);
}

test('the_settings_subnav_and_the_content_stay_in_place_between_a_list_and_a_record', async ({
	page
}) => {
	for (const width of WIDTHS) {
		await page.setViewportSize({ width, height: 900 });
		await signIn(page);

		for (const path of ['/settings/scripts/templates', '/settings/scripts/jobs']) {
			await openList(page, path);
			// The list fits the window, so it shows no window scrollbar.
			expect(await overflows(page), `${path} at ${width} px: the list fills the window`).toBe(
				false
			);
			const list = await place(page);

			await openRecord(page, path);
			expectSamePlace(list, await place(page), `${path} at ${width} px`);
			// The record fills more than the window, so it shows the window scrollbar.
			expect(await overflows(page), `${path} at ${width} px: the record fits the window`).toBe(
				true
			);

			await page.locator(`a[href="${path}"]`).first().click();
			await expect(page).toHaveURL(new RegExp(`${path}(\\?.*)?$`));
			await expect(page.locator('li.item').first()).toBeVisible();
			expectSamePlace(list, await place(page), `${path} back at ${width} px`);
		}
	}
});

test('the_content_column_keeps_its_width_when_both_pages_show_the_window_scrollbar', async ({
	page
}) => {
	for (const width of WIDTHS) {
		// A short window shows the window scrollbar on the list too.
		await page.setViewportSize({ width, height: 480 });
		await signIn(page);

		for (const path of ['/settings/scripts/templates', '/settings/scripts/jobs']) {
			await openList(page, path);
			expect(await overflows(page), `${path} at ${width} px: the list fits the window`).toBe(true);
			const list = await place(page);

			await openRecord(page, path);
			expect(await overflows(page), `${path} at ${width} px: the record fits the window`).toBe(
				true
			);
			const record = await place(page);
			expectSamePlace(list, record, `${path} at ${width} px`);
			expectSameWidth(list, record, `${path} at ${width} px`);
		}
	}
});

test('a settings record does not scroll sideways in the content column', async ({ page }) => {
	const PATHS = ['/settings/scripts/templates', '/settings/scripts/jobs', '/settings/access/users'];

	for (const width of WIDTHS) {
		await page.setViewportSize({ width, height: 900 });
		await signIn(page);

		for (const path of PATHS) {
			await openList(page, path);
			await openRecord(page, path);
			const wide = await page.evaluate(() => {
				const root = document.scrollingElement as Element;
				const inside = ['.settings', '.settings__content', '.record', '.record__main']
					.map((selector) => document.querySelector<HTMLElement>(selector))
					.filter((element): element is HTMLElement => element !== null)
					.filter((element) => element.scrollWidth > element.clientWidth + 1)
					.map((element) => element.className);
				return { page: root.scrollWidth > root.clientWidth + 1, inside };
			});
			expect(wide, `${path} at ${width} px scrolls sideways`).toEqual({ page: false, inside: [] });
		}
	}
});
