import { expect, test, type Page } from '@playwright/test';

// The production policy is `style-src 'self'` without `unsafe-inline` (ADR-0027).
// The test fails on any `securitypolicyviolation` event and on the CSP console error.

// Registers both collectors. The console messages collect on the Node side. The
// `securitypolicyviolation` events collect in the page, because an event can arrive after the
// assertion that triggered it.
async function watch(page: Page): Promise<string[]> {
	const consoleViolations: string[] = [];
	page.on('console', (message) => {
		if (/Content Security Policy/i.test(message.text())) {
			consoleViolations.push(message.text());
		}
	});
	await page.addInitScript(() => {
		const violations: string[] = [];
		(window as unknown as { __cspViolations: string[] }).__cspViolations = violations;
		document.addEventListener('securitypolicyviolation', (event) => {
			// Chromium reports a violation of a style attribute in a template as a Document
			// target, so the target is not always an element.
			const target = event.target;
			const element = target instanceof Element ? target : null;
			const detail = element
				? element.tagName + ' style=' + (element.getAttribute('style') ?? '')
				: String(target);
			violations.push(event.violatedDirective + ' | ' + detail);
		});
	});
	return consoleViolations;
}

// Reads the page-side violations after a settle step. One task plus a frame lets a pending
// `securitypolicyviolation` event reach the listener before the assertion.
async function pageViolations(page: Page): Promise<string[]> {
	return page.evaluate(
		() =>
			new Promise<string[]>((resolve) => {
				setTimeout(() => {
					requestAnimationFrame(() =>
						resolve((window as unknown as { __cspViolations: string[] }).__cspViolations)
					);
				}, 0);
			})
	);
}

test('the built page carries the strict policy', async ({ page }) => {
	const consoleViolations = await watch(page);
	const response = await page.goto('/sign-in');
	// The heading names the sign-in screen, so a redirect to the setup screen fails here.
	await expect(page.getByRole('heading', { level: 1, name: 'Sign in' })).toBeVisible();

	const policy = response?.headers()['content-security-policy'] ?? '';
	expect(policy).toContain("style-src 'self'");
	expect(policy).toContain("style-src-attr 'unsafe-hashes' 'sha256-");
	expect(policy).not.toContain('unsafe-inline');

	// The password toggle is a component; it must render without an inline style.
	const password = page
		.locator('.rh-input-affix')
		.filter({ has: page.locator('#signin-password') });
	await password.getByRole('button', { name: 'Show' }).click();
	await expect(password.locator('#signin-password')).toHaveAttribute('type', 'text');
	expect(await pageViolations(page)).toEqual([]);
	expect(consoleViolations).toEqual([]);
});

test('the floating primitives need no inline style', async ({ page }) => {
	const consoleViolations = await watch(page);
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.getByRole('button', { name: 'Sign in', exact: true }).click();

	// The shell renders after the sign-in.
	await expect(page.getByRole('navigation', { name: 'Primary navigation' })).toBeVisible();

	// The account menu is a bits-ui dropdown; opening it mounts the floating layer.
	await page.locator('.rh-menu__trigger').click();
	await expect(page.getByRole('menuitem', { name: 'Sign out' })).toBeVisible();

	expect(await pageViolations(page)).toEqual([]);
	expect(consoleViolations).toEqual([]);
});

test('the icons and an open dialog need no inline style', async ({ page }) => {
	const consoleViolations = await watch(page);
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.getByRole('button', { name: 'Sign in', exact: true }).click();
	await expect(page.getByRole('navigation', { name: 'Primary navigation' })).toBeVisible();

	// The shell and the overview draw the icons as inline SVG elements, not as a sprite reference.
	const nav = page.getByRole('navigation', { name: 'Primary navigation' });
	await expect(nav.locator('svg.icon[data-icon="overview"]')).toBeVisible();
	await expect(page.locator('main svg.icon[data-icon="host"]')).toBeVisible();

	// The organization switcher marks the current organization and the platform entry with an icon.
	await page.getByRole('button', { name: 'Switch organization' }).click();
	await expect(
		page.getByRole('menuitemradio', { name: /acme/ }).locator('svg.icon[data-icon="check"]')
	).toBeVisible();
	await expect(
		page
			.getByRole('menuitem', { name: 'Platform administration' })
			.locator('svg.icon[data-icon="shield"]')
	).toBeVisible();
	await page.keyboard.press('Escape');

	await page
		.getByRole('navigation', { name: 'Primary navigation' })
		.getByRole('link', { name: 'Audit', exact: true })
		.click();
	await page.getByRole('button', { name: 'Details' }).first().click();
	const panel = page.getByRole('dialog');
	await expect(panel).toHaveClass(/rh-panel/);
	await expect(panel.locator('svg.icon[data-icon="x"]')).toBeVisible();
	await page.keyboard.press('Escape');
	await expect(panel).toBeHidden();

	// The API tokens screen lives under Settings, Access.
	await page
		.getByRole('navigation', { name: 'Primary navigation' })
		.getByRole('link', { name: 'Settings' })
		.click();
	await page
		.getByRole('navigation', { name: 'Settings sections' })
		.getByRole('link', { name: 'API tokens' })
		.click();
	await expect(page).toHaveURL(/\/settings\/access\/api-tokens$/);
	await page.getByRole('button', { name: 'Create token' }).click();
	const dialog = page.getByRole('dialog', { name: 'Create an API token' });
	await expect(dialog).toHaveClass(/rh-dialog/);
	await expect(dialog.getByRole('button', { name: 'Close' })).toBeVisible();

	expect(await pageViolations(page)).toEqual([]);
	expect(consoleViolations).toEqual([]);
});

// The mock keeps the session in memory, so the test follows a link instead of a page load.
async function follow(page: Page, href: string): Promise<void> {
	await page.evaluate((target) => {
		const link = document.createElement('a');
		link.href = target;
		link.id = 'e2e-old-link';
		link.textContent = target;
		document.body.append(link);
	}, href);
	const link = page.locator('#e2e-old-link');
	await link.click();
	await link.evaluate((element) => element.remove());
}

test('the old users and API tokens addresses open the settings screens', async ({ page }) => {
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.getByRole('button', { name: 'Sign in', exact: true }).click();
	await expect(page.getByRole('navigation', { name: 'Primary navigation' })).toBeVisible();

	await follow(page, '/api-tokens');
	await expect(page).toHaveURL(/\/settings\/access\/api-tokens$/);
	await expect(page.getByRole('heading', { level: 1, name: 'API tokens' })).toBeVisible();

	await follow(page, '/users');
	await expect(page).toHaveURL(/\/settings\/access\/users$/);
	await expect(page.getByRole('heading', { level: 1, name: 'Users' })).toBeVisible();
	await expect(page).toHaveTitle('Users · ResticHub');
});

test('the retention inputs keep the app input style under the policy', async ({ page }) => {
	const consoleViolations = await watch(page);
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.getByRole('button', { name: 'Sign in', exact: true }).click();
	const nav = page.getByRole('navigation', { name: 'Primary navigation' });
	await expect(nav).toBeVisible();

	await nav.getByRole('link', { name: 'Profiles', exact: true }).click();
	await page.getByRole('button', { name: 'Add profile', exact: true }).first().click();
	await page.locator('[data-section="retention"] .section__toggle').click();

	const inputs = page.locator('[data-section="retention"] .rule__input input');
	await expect(inputs).toHaveCount(7);
	const boxes = [];
	for (const input of await inputs.all()) {
		await expect(input).toHaveClass(/rh-input/);
		const style = await input.evaluate((element) => {
			const computed = getComputedStyle(element);
			return {
				border: computed.borderTopColor,
				padding: computed.paddingLeft,
				radius: computed.borderTopLeftRadius
			};
		});
		expect(style.padding).not.toBe('0px');
		expect(style.radius).not.toBe('0px');
		expect(style.border).not.toBe('rgb(0, 0, 0)');
		boxes.push(await input.boundingBox());
	}
	// One width and one left edge for every rule input.
	expect(new Set(boxes.map((box) => Math.round(box?.width ?? 0))).size).toBe(1);
	expect(new Set(boxes.map((box) => Math.round(box?.x ?? 0))).size).toBe(1);

	expect(await pageViolations(page)).toEqual([]);
	expect(consoleViolations).toEqual([]);
});
