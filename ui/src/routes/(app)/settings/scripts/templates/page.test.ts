/*
 * The Script templates screen lists the built-in and the own templates, opens each one on its page
 * and deletes an own template (FR-HK-12).
 */
import { render, screen, waitFor, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import Templates from './+page.svelte';

const mocks = vi.hoisted(() => ({
	listScriptTemplates: vi.fn(),
	createScriptTemplate: vi.fn(),
	updateScriptTemplate: vi.fn(),
	deleteScriptTemplate: vi.fn(),
	pushToast: vi.fn(),
	goto: vi.fn()
}));

vi.mock('#lib/stores/session.svelte.ts', () => ({
	getSession: () => ({
		user: { id: 'u_01', username: 'admin', display_name: 'Anna', role: 'admin' },
		tenant: { id: 't_01', name: 'acme' },
		role: 'admin'
	})
}));

vi.mock('#lib/stores/toasts.svelte.ts', () => ({ pushToast: mocks.pushToast }));

const route = vi.hoisted(() => ({ url: new URL('http://localhost/settings/scripts/templates') }));

vi.mock('$app/state', () => ({ page: route }));
vi.mock('$app/paths', () => ({
	resolve: (path: string, params?: Record<string, string>) =>
		params ? path.replace('[id]', params.id).replace('/(app)', '') : path.replace('/(app)', '')
}));
vi.mock('$app/navigation', () => ({
	goto: (address: string) => {
		mocks.goto(address);
		route.url = new URL(address, 'http://localhost');
	}
}));

vi.mock('#lib/api/index.ts', async (importOriginal) => {
	const actual = await importOriginal<typeof import('#lib/api/index.ts')>();
	return {
		...actual,
		service: {
			...actual.service,
			listScriptTemplates: mocks.listScriptTemplates,
			createScriptTemplate: mocks.createScriptTemplate,
			updateScriptTemplate: mocks.updateScriptTemplate,
			deleteScriptTemplate: mocks.deleteScriptTemplate
		}
	};
});

function template(overrides: Record<string, unknown> = {}) {
	return {
		id: 'builtin:disk-space-check-bash',
		builtin: true,
		name: 'Free disk space check (Linux)',
		description: 'Fails when the path has less free space than the limit.',
		interpreter: 'bash',
		phase: 'before',
		body: 'df -h "$CHECK_PATH"\n',
		timeout_seconds: 60,
		failure_policy: 'abort_backup',
		parameters: [{ name: 'CHECK_PATH', label: 'Path', type: 'string', default: '/' }],
		...overrides
	};
}

const OWN = template({
	id: 'st_1',
	builtin: false,
	name: 'Remove old dumps',
	description: 'Deletes old dump files.',
	phase: 'finally',
	parameters: []
});

beforeEach(() => {
	route.url = new URL('http://localhost/settings/scripts/templates');
	for (const mock of Object.values(mocks)) mock.mockReset();
	mocks.listScriptTemplates.mockResolvedValue([template(), OWN]);
	mocks.createScriptTemplate.mockResolvedValue(OWN);
	mocks.updateScriptTemplate.mockResolvedValue(OWN);
	mocks.deleteScriptTemplate.mockResolvedValue(undefined);
});

// The own templates open first; a built-in template is on its own tab.
async function rowOf(name: string, builtin = false): Promise<HTMLElement> {
	if (builtin) await userEvent.click(await screen.findByRole('button', { name: 'Built-in 1' }));
	return (await screen.findByText(name)).closest('li') as HTMLElement;
}

describe('Script templates screen', () => {
	it('lists the built-in and the own templates with their source', async () => {
		render(Templates);
		expect(await screen.findByText('Remove old dumps')).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Own 1' })).toHaveAttribute('aria-pressed', 'true');
		expect(screen.queryByText('Free disk space check (Linux)')).toBeNull();
		await rowOf('Free disk space check (Linux)', true);
		expect(screen.getByText('built-in')).toBeInTheDocument();
	});

	it('offers no delete action for a built-in template', async () => {
		render(Templates);
		const row = await rowOf('Free disk space check (Linux)', true);
		expect(within(row).getByRole('button', { name: 'Make a copy' })).toBeInTheDocument();
		expect(within(row).queryByRole('button', { name: 'Delete' })).not.toBeInTheDocument();
	});

	it('links each template to its page and opens a copy as a new template', async () => {
		const user = userEvent.setup();
		render(Templates);
		expect(await screen.findByRole('link', { name: 'Remove old dumps' })).toHaveAttribute(
			'href',
			'/settings/scripts/templates/st_1'
		);
		const row = await rowOf('Free disk space check (Linux)', true);
		await user.click(within(row).getByRole('button', { name: 'Make a copy' }));
		expect(mocks.goto).toHaveBeenCalledWith(
			'/settings/scripts/templates/new?from=builtin%3Adisk-space-check-bash'
		);
		await user.click(screen.getByRole('button', { name: 'Add template' }));
		expect(mocks.goto).toHaveBeenLastCalledWith('/settings/scripts/templates/new');
	});

	it('deletes an own template after a confirmation', async () => {
		const user = userEvent.setup();
		render(Templates);
		await user.click(
			within(await rowOf('Remove old dumps')).getByRole('button', { name: 'Delete' })
		);
		const dialog = await screen.findByRole('dialog', { name: 'Delete the template?' });
		expect(within(dialog).getByText(/The profiles keep the scripts/)).toBeInTheDocument();
		await user.click(within(dialog).getByRole('button', { name: 'Delete' }));
		await waitFor(() => expect(mocks.deleteScriptTemplate).toHaveBeenCalledWith('st_1'));
	});
});
