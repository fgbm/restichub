/*
 * The script template page shows a built-in template read-only with a copy action, creates a
 * template with its parameters, shows a field problem and deletes an own template (FR-HK-12).
 */
import { render, screen, waitFor, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import ScriptTemplateEditor from './ScriptTemplateEditor.svelte';

const mocks = vi.hoisted(() => ({
	listScriptTemplates: vi.fn(),
	createScriptTemplate: vi.fn(),
	updateScriptTemplate: vi.fn(),
	deleteScriptTemplate: vi.fn(),
	pushToast: vi.fn(),
	goto: vi.fn()
}));

const session = vi.hoisted(() => ({ role: 'admin' as string }));

vi.mock('#lib/stores/session.svelte.ts', () => ({
	getSession: () => ({
		user: { id: 'u_01', username: 'admin', display_name: 'Anna', role: session.role },
		tenant: { id: 't_01', name: 'acme' },
		role: session.role
	})
}));

vi.mock('#lib/stores/toasts.svelte.ts', () => ({ pushToast: mocks.pushToast }));

const route = vi.hoisted(() => ({ url: new URL('http://localhost/settings/scripts/templates') }));

vi.mock('$app/state', () => ({ page: route }));
vi.mock('$app/paths', () => ({
	resolve: (path: string) => path.replace('/(app)', '')
}));
vi.mock('$app/navigation', () => ({ goto: mocks.goto, beforeNavigate: vi.fn() }));

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
	session.role = 'admin';
	for (const mock of Object.values(mocks)) mock.mockReset();
	mocks.listScriptTemplates.mockResolvedValue([template(), OWN]);
	mocks.createScriptTemplate.mockResolvedValue(OWN);
	mocks.updateScriptTemplate.mockResolvedValue(OWN);
	mocks.deleteScriptTemplate.mockResolvedValue(undefined);
});

describe('Script template page', () => {
	it('shows a built-in template read-only and opens a copy as a new template', async () => {
		const user = userEvent.setup();
		render(ScriptTemplateEditor, { templateId: 'builtin:disk-space-check-bash' });
		expect(
			await screen.findByRole('heading', { level: 1, name: 'Free disk space check (Linux)' })
		).toBeInTheDocument();
		expect(
			screen.getByText('A built-in template is read-only. Make a copy to change it.')
		).toBeInTheDocument();
		expect(document.querySelector('#template-name')).toHaveAttribute('readonly');
		expect(screen.queryByRole('button', { name: 'Save' })).toBeNull();
		expect(screen.queryByRole('button', { name: 'Delete' })).toBeNull();
		// The side panel names the environment variables of the parameters.
		const variables = screen
			.getByRole('heading', { name: 'Environment variables' })
			.closest('section');
		expect(within(variables as HTMLElement).getByText('CHECK_PATH')).toBeInTheDocument();
		await user.click(screen.getByRole('button', { name: 'Make a copy' }));
		expect(mocks.goto).toHaveBeenCalledWith(
			'/settings/scripts/templates/new?from=builtin%3Adisk-space-check-bash'
		);
	});

	it('saves a copy of a built-in template as a new template', async () => {
		const user = userEvent.setup();
		render(ScriptTemplateEditor, { copyFrom: 'builtin:disk-space-check-bash' });
		await screen.findByRole('heading', { level: 1, name: 'New script template' });
		expect(document.querySelector('#template-name')).toHaveValue(
			'Free disk space check (Linux) copy'
		);
		await user.click(screen.getByRole('button', { name: 'Save' }));
		await waitFor(() => expect(mocks.createScriptTemplate).toHaveBeenCalledTimes(1));
		expect(mocks.createScriptTemplate.mock.calls[0][0]).toMatchObject({
			name: 'Free disk space check (Linux) copy',
			interpreter: 'bash',
			phase: 'before',
			timeout_seconds: 60,
			failure_policy: 'abort_backup',
			parameters: [{ name: 'CHECK_PATH', label: 'Path', type: 'string', default: '/' }]
		});
		expect(mocks.goto).toHaveBeenCalledWith('/settings/scripts/templates');
	});

	it('creates a template with a parameter', async () => {
		const user = userEvent.setup();
		render(ScriptTemplateEditor);
		await screen.findByRole('heading', { level: 1, name: 'New script template' });
		await user.type(document.querySelector('#template-name') as HTMLElement, 'Stop service');
		await user.type(screen.getByLabelText(/^Script/), 'systemctl stop "$SERVICE"');
		await user.click(screen.getByRole('button', { name: 'Add parameter' }));
		await user.type(screen.getByLabelText('Variable'), 'SERVICE');
		await user.click(screen.getByRole('button', { name: 'Save' }));
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
		render(ScriptTemplateEditor);
		await screen.findByRole('heading', { level: 1, name: 'New script template' });
		await user.type(document.querySelector('#template-name') as HTMLElement, 'Remove old dumps');
		await user.type(screen.getByLabelText(/^Script/), 'true');
		await user.click(screen.getByRole('button', { name: 'Save' }));
		expect(await screen.findByRole('alert')).toBeInTheDocument();
		expect(mocks.goto).not.toHaveBeenCalled();
	});

	it('keeps the delete of an own template in the dangerous section only', async () => {
		render(ScriptTemplateEditor, { templateId: 'st_1' });
		await screen.findByRole('heading', { level: 1, name: 'Remove old dumps' });

		const danger = screen
			.getByRole('button', { name: /Dangerous actions/ })
			.closest('section') as HTMLElement;
		expect(danger).toHaveAttribute('data-section', 'danger');
		expect(
			within(danger).getByText(/The profiles keep the scripts that they copied/)
		).toBeInTheDocument();
		expect(within(danger).getByRole('button', { name: 'Delete' })).toHaveClass('btn--danger');
		const header = document.querySelector<HTMLElement>('.page-header__actions') as HTMLElement;
		expect(within(header).queryByRole('button', { name: 'Delete' })).toBeNull();
	});

	it('shows no dangerous section for a built-in template, a new template and a viewer', async () => {
		const builtin = render(ScriptTemplateEditor, { templateId: 'builtin:disk-space-check-bash' });
		await screen.findByRole('heading', { level: 1, name: 'Free disk space check (Linux)' });
		expect(document.querySelector('[data-section="danger"]')).toBeNull();
		builtin.unmount();

		const created = render(ScriptTemplateEditor);
		await screen.findByRole('heading', { level: 1, name: 'New script template' });
		expect(document.querySelector('[data-section="danger"]')).toBeNull();
		created.unmount();

		session.role = 'viewer';
		render(ScriptTemplateEditor, { templateId: 'st_1' });
		await screen.findByRole('heading', { level: 1, name: 'Remove old dumps' });
		expect(document.querySelector('[data-section="danger"]')).toBeNull();
	});

	it('deletes an own template after a confirmation and returns to the list', async () => {
		const user = userEvent.setup();
		render(ScriptTemplateEditor, { templateId: 'st_1' });
		await screen.findByRole('heading', { level: 1, name: 'Remove old dumps' });
		await user.click(screen.getByRole('button', { name: 'Delete' }));
		const dialog = await screen.findByRole('dialog', { name: 'Delete the template?' });
		await user.click(within(dialog).getByRole('button', { name: 'Delete' }));
		await waitFor(() => expect(mocks.deleteScriptTemplate).toHaveBeenCalledWith('st_1'));
		expect(mocks.goto).toHaveBeenCalledWith('/settings/scripts/templates');
	});
});
