import { expect, test, type Locator, type Page, type TestInfo } from '@playwright/test';
import { pathToFileURL } from 'node:url';
import { resolve } from 'node:path';

// The comparison reads the geometry and the computed style of the shell and of the key screens
// in the app and in the static mockups of `ui/mockups`. A pixel comparison would fail on the
// sample data and on the fonts of the host, so the check compares what the design fixes:
// sizes, colors and weights. Both screenshots go to the report for a visual review.
test.skip(({ browserName }) => browserName !== 'chromium', 'one engine renders the comparison');
test.use({ viewport: { width: 1440, height: 900 }, colorScheme: 'light' });

interface Measure {
	height: number;
	width: number;
	fontSize: string;
	fontWeight: string;
	color: string;
	background: string;
	radius: string;
}

async function measure(locator: Locator): Promise<Measure> {
	return locator.first().evaluate((element) => {
		const box = element.getBoundingClientRect();
		const style = getComputedStyle(element);
		return {
			height: Math.round(box.height),
			width: Math.round(box.width),
			fontSize: style.fontSize,
			fontWeight: style.fontWeight,
			color: style.color,
			background: style.backgroundColor,
			radius: style.borderTopLeftRadius
		};
	});
}

function mockupUrl(name: string): string {
	return pathToFileURL(resolve('mockups', name)).href;
}

async function signIn(page: Page): Promise<void> {
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.getByRole('button', { name: 'Sign in', exact: true }).click();
	await expect(page.getByRole('navigation', { name: 'Primary navigation' })).toBeVisible();
}

async function openFromNav(page: Page, name: string): Promise<void> {
	await page
		.getByRole('navigation', { name: 'Primary navigation' })
		.getByRole('link', { name, exact: true })
		.click();
	await page.getByRole('heading', { level: 1 }).waitFor();
}

// Both sides carry the same fields; `skip` lists a field that the design lets differ.
function expectSame(app: Measure, mockup: Measure, skip: (keyof Measure)[] = []): void {
	for (const key of Object.keys(mockup) as (keyof Measure)[]) {
		if (skip.includes(key)) continue;
		expect(app[key], key).toEqual(mockup[key]);
	}
}

async function attach(page: Page, name: string, info: TestInfo): Promise<void> {
	await info.attach(name, { body: await page.screenshot(), contentType: 'image/png' });
}

test('the shell matches the mockup', async ({ page }, info) => {
	await page.goto(mockupUrl('hosts.html'));
	const mockup = {
		topbar: await measure(page.locator('header.topbar')),
		title: await measure(page.locator('h1.page-title')),
		page: await measure(page.locator('main.page'))
	};
	await attach(page, 'mockup-shell', info);

	await signIn(page);
	await openFromNav(page, 'Hosts');
	const app = {
		topbar: await measure(page.locator('header').first()),
		title: await measure(page.getByRole('heading', { level: 1 })),
		page: await measure(page.locator('main'))
	};
	await attach(page, 'app-shell', info);

	expectSame(app.topbar, mockup.topbar, ['width']);
	expectSame(app.title, mockup.title, ['width', 'height']);
	expect(app.page.width).toBeLessThanOrEqual(mockup.page.width);
});

for (const [screen, file, link] of [
	['hosts', 'hosts.html', 'Hosts'],
	['settings', 'settings.html', 'Settings']
] as const) {
	test(`the ${screen} title matches the mockup`, async ({ page }, info) => {
		await page.goto(mockupUrl(file));
		const mockup = await measure(page.locator('h1.page-title'));
		await attach(page, `mockup-${screen}`, info);

		await signIn(page);
		await openFromNav(page, link);
		const app = await measure(page.getByRole('heading', { level: 1 }));
		await attach(page, `app-${screen}`, info);

		expectSame(app, mockup, ['width', 'height']);
	});
}

test('the users table matches the mockup', async ({ page }, info) => {
	await page.goto(mockupUrl('users.html'));
	const mockup = {
		head: await measure(page.locator('th').first()),
		button: await measure(page.locator('.btn--primary').first())
	};
	await attach(page, 'mockup-users', info);

	await signIn(page);
	await openFromNav(page, 'Settings');
	await page
		.getByRole('navigation', { name: 'Settings sections' })
		.getByRole('link', { name: 'Users' })
		.click();
	await page.getByRole('heading', { level: 1, name: 'Users' }).waitFor();
	await page.locator('th').first().waitFor();
	const app = {
		head: await measure(page.locator('th').first()),
		button: await measure(page.locator('.btn--primary').first())
	};
	await attach(page, 'app-users', info);

	expectSame(app.head, mockup.head, ['width', 'height']);
	expectSame(app.button, mockup.button, ['width']);
});

test('the sign-in screen matches the mockup', async ({ page }, info) => {
	await page.goto(mockupUrl('sign-in.html'));
	const mockup = {
		title: await measure(page.locator('h1')),
		button: await measure(page.locator('.btn--primary').first())
	};
	await attach(page, 'mockup-sign-in', info);

	await page.goto('/sign-in');
	await page.getByRole('heading', { level: 1 }).waitFor();
	const app = {
		title: await measure(page.getByRole('heading', { level: 1 })),
		button: await measure(page.locator('.btn--primary').first())
	};
	await attach(page, 'app-sign-in', info);

	expectSame(app.title, mockup.title, ['width', 'height']);
	expectSame(app.button, mockup.button, ['width']);
});
