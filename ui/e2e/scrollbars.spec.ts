import { expect, test, type Page } from '@playwright/test';

// With classic scrollbars the window scrollbar takes 15 px of the window. The fixed layers must
// reach the window edge there too, and the middle of the window holds the middle of the dialog.
// The project `chromium-classic-scrollbars` runs this file (playwright.config.ts).

async function signIn(page: Page): Promise<void> {
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.getByRole('button', { name: 'Sign in', exact: true }).click();
	await expect(page.getByRole('navigation', { name: 'Primary navigation' })).toBeVisible();
}

// A reload drops the mock session, so the test follows the links of the shell.
async function openSettings(page: Page, path: string): Promise<void> {
	await page.locator('a[href="/settings"]').first().click();
	await page.locator(`a[href="${path}"]`).first().click();
	await expect(page.locator('li.item').first()).toBeVisible();
}

async function openPasswordDialog(page: Page): Promise<void> {
	await page.locator('header .rh-menu__trigger').click();
	await page.getByRole('menuitem', { name: 'Change password' }).click();
	await expect(page.locator('.rh-dialog')).toBeVisible();
}

async function overflows(page: Page): Promise<boolean> {
	return page.evaluate(() => {
		const root = document.scrollingElement as Element;
		return root.scrollHeight > root.clientHeight;
	});
}

interface WindowLayout {
	innerWidth: number;
	clientWidth: number;
	overlayRight: number;
	dialogCentre: number;
}

async function windowLayout(page: Page): Promise<WindowLayout> {
	return page.evaluate(() => {
		const overlay = document.querySelector('.rh-dialog-overlay')?.getBoundingClientRect();
		const dialog = document.querySelector('.rh-dialog')?.getBoundingClientRect();
		return {
			innerWidth: window.innerWidth,
			clientWidth: document.documentElement.clientWidth,
			overlayRight: overlay?.right ?? 0,
			dialogCentre: dialog ? dialog.x + dialog.width / 2 : 0
		};
	});
}

function expectWindowLayout(layout: WindowLayout, where: string): void {
	expect(layout.overlayRight, `${where}: the dialog overlay stops before the window edge`).toBe(
		layout.innerWidth
	);
	expect(
		Math.abs(layout.dialogCentre - layout.innerWidth / 2),
		`${where}: the dialog is not in the middle of the window`
	).toBeLessThanOrEqual(1);
}

test('the_dialog_reaches_the_window_edge_on_a_page_that_does_not_scroll', async ({ page }) => {
	await page.setViewportSize({ width: 1280, height: 1600 });
	await signIn(page);
	await openSettings(page, '/settings/scripts/templates');
	await openPasswordDialog(page);

	const layout = await windowLayout(page);
	expect(layout.clientWidth, 'the short page keeps a strip for the window scrollbar').toBe(
		layout.innerWidth
	);
	expectWindowLayout(layout, 'the short page');
});

test('the_dialog_reaches_the_window_edge_on_a_page_that_scrolls', async ({ page }) => {
	await page.setViewportSize({ width: 1280, height: 480 });
	await signIn(page);
	await openSettings(page, '/settings/scripts/templates');
	expect(await overflows(page), 'the page fits the window').toBe(true);
	await openPasswordDialog(page);

	expectWindowLayout(await windowLayout(page), 'the scrolling page');
});

test('the_side_panel_reaches_the_window_edge', async ({ page }) => {
	await page.setViewportSize({ width: 1280, height: 320 });
	await page.addInitScript(() => {
		localStorage.setItem('restichub.mock.alerts', 'sample');
	});
	await signIn(page);
	await page
		.getByRole('navigation', { name: 'Primary navigation' })
		.getByRole('link', { name: 'Alerts', exact: true })
		.click();
	await expect(page.locator('li.item').first()).toBeVisible();
	expect(await overflows(page), 'the page fits the window').toBe(true);

	await page.locator('li.item .item__title').first().click();
	await expect(page.locator('.rh-panel')).toBeVisible();
	const layout = await page.evaluate(() => {
		const panel = document.querySelector('.rh-panel')?.getBoundingClientRect();
		return { right: panel?.right ?? 0, innerWidth: window.innerWidth };
	});
	expect(layout.right, 'the side panel stops before the window edge').toBe(layout.innerWidth);
});
