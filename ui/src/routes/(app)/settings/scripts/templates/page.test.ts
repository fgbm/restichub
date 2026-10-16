/*
 * The Script templates screen lists the built-in and the own templates, opens a built-in template
 * read-only and makes a copy of it, and creates, edits and deletes an own template (FR-HK-12).
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
	pushToast: vi.fn()
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
vi.mock('$app/navigation', () => ({
	goto: (address: string) => {
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

	it('opens a built-in template read-only and copies it into a new template', async () => {
		const user = userEvent.setup();
		render(Templates);
		// The row opens the template; opening is not a row action.
		await user.click(
			within(await rowOf('Free disk space check (Linux)', true)).getByRole('button', {
				name: 'Free disk space check (Linux)'
			})
		);
		const dialog = await screen.findByRole('dialog');
		expect(
			within(dialog).getByText('A built-in template is read-only. Make a copy to change it.')
		).toBeInTheDocument();
		expect(within(dialog).getByLabelText(/^Template/)).toHaveAttribute('readonly');
		await user.click(within(dialog).getByRole('button', { name: 'Make a copy' }));
		const copy = await screen.findByRole('dialog');
		await waitFor(() =>
			expect(within(copy).getByLabelText(/^Template/)).toHaveValue(
				'Free disk space check (Linux) copy'
			)
		);
		expect(within(copy).getByLabelText(/^Template/)).not.toHaveAttribute('readonly');
		await user.click(within(copy).getByRole('button', { name: 'Save' }));
		await waitFor(() => expect(mocks.createScriptTemplate).toHaveBeenCalledTimes(1));
		const request = mocks.createScriptTemplate.mock.calls[0][0];
		expect(request).toMatchObject({
			name: 'Free disk space check (Linux) copy',
			interpreter: 'bash',
			phase: 'before',
			timeout_seconds: 60,
			failure_policy: 'abort_backup',
			parameters: [{ name: 'CHECK_PATH', label: 'Path', type: 'string', default: '/' }]
		});
	});

	it('creates a template with a parameter', async () => {
		const user = userEvent.setup();
		render(Templates);
		await user.click(await screen.findByRole('button', { name: 'Add template' }));
		const dialog = await screen.findByRole('dialog');
		await user.type(within(dialog).getByLabelText(/^Template/), 'Stop service');
		await user.type(within(dialog).getByLabelText(/^Script/), 'systemctl stop "$SERVICE"');
		await user.click(within(dialog).getByRole('button', { name: 'Add parameter' }));
		await user.type(within(dialog).getByLabelText('Variable'), 'SERVICE');
		await user.click(within(dialog).getByRole('button', { name: 'Save' }));
		await waitFor(() => expect(mocks.createScriptTemplate).toHaveBeenCalledTimes(1));
		expect(mocks.createScriptTemplate.mock.calls[0][0]).toMatchObject({
			name: 'Stop service',
			body: 'systemctl stop "$SERVICE"',
			parameters: [{ name: 'SERVICE', label: 'SERVICE', type: 'string' }]
		});
		expect(mocks.pushToast).toHaveBeenCalledWith('success', 'The template is created.');
	});

	it('shows the field problem of the control plane next to the field', async () => {
		const user = userEvent.setup();
		mocks.createScriptTemplate.mockRejectedValue(
			Object.assign(new Error('name'), {
				problem: {
					status: 409,
					code: 'conflict',
					title: 'Conflict',
					field: 'name',
					reason: 'unknown'
				}
			})
		);
		render(Templates);
		await user.click(await screen.findByRole('button', { name: 'Add template' }));
		const dialog = await screen.findByRole('dialog');
		await user.type(within(dialog).getByLabelText(/^Template/), 'Remove old dumps');
		await user.type(within(dialog).getByLabelText(/^Script/), 'true');
		await user.click(within(dialog).getByRole('button', { name: 'Save' }));
		expect(await within(dialog).findByRole('alert')).toBeInTheDocument();
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
