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

test('the external icon sprite needs no inline style', async ({ page }) => {
	const consoleViolations = await watch(page);
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.getByRole('button', { name: 'Sign in', exact: true }).click();

	// The overview renders sprite icons: the welcome banner, the steps and the empty states.
	await expect(page.getByRole('heading', { level: 1, name: 'Overview' })).toBeVisible();
	const icon = page.locator('svg.icon use').first();
	await expect(icon).toHaveAttribute('href', /\/icons\.svg#/);

	// The symbol paints: an external `<use>` resolves under `default-src 'self'`.
	const rendered = await icon.evaluate((element) => {
		const box = (element as unknown as SVGGraphicsElement).getBBox();
		return box.width + box.height;
	});
	expect(rendered).toBeGreaterThan(0);

	expect(await pageViolations(page)).toEqual([]);
	expect(consoleViolations).toEqual([]);
});
