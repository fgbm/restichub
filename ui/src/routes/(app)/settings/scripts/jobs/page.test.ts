/*
 * The Job templates screen lists the built-in and the own templates, links each one to its page and
 * deletes an own template (FR-HK-12). An Operator reads the templates and changes nothing, because
 * the API refuses every template write of that role (AC-AU-02).
 */
import { render, screen, waitFor, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiError } from '#lib/api/index.ts';
import Templates from './+page.svelte';

const mocks = vi.hoisted(() => ({
	listJobTemplates: vi.fn(),
	listScriptTemplates: vi.fn(),
	listSecrets: vi.fn(),
	createJobTemplate: vi.fn(),
	updateJobTemplate: vi.fn(),
	deleteJobTemplate: vi.fn(),
	pushToast: vi.fn(),
	goto: vi.fn()
}));

const session = vi.hoisted(() => ({ role: 'admin' }));

vi.mock('#lib/stores/session.svelte.ts', () => ({
	getSession: () => ({
		user: { id: 'u_01', username: 'admin', display_name: 'Anna', role: session.role },
		tenant: { id: 't_01', name: 'acme' },
		role: session.role
	}),
	hasRole: (...roles: string[]) => roles.includes(session.role)
}));

vi.mock('#lib/stores/toasts.svelte.ts', () => ({ pushToast: mocks.pushToast }));

const route = vi.hoisted(() => ({ url: new URL('http://localhost/settings/scripts/jobs') }));

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
			listJobTemplates: mocks.listJobTemplates,
			listScriptTemplates: mocks.listScriptTemplates,
			listSecrets: mocks.listSecrets,
			createJobTemplate: mocks.createJobTemplate,
			updateJobTemplate: mocks.updateJobTemplate,
			deleteJobTemplate: mocks.deleteJobTemplate
		}
	};
});

function script(name: string, phase: string) {
	return {
		name,
		interpreter: 'bash',
		phase,
		body: `echo ${name}\n`,
		timeout_seconds: 60,
		failure_policy: 'abort_backup',
		env: [{ name: 'PGPASSWORD', kind: 'secret_ref', value: '' }]
	};
}

const BUILTIN = {
	id: 'builtin:postgres-linux',
	builtin: true,
	name: 'PostgreSQL server (Linux)',
	description: 'Checks the free space, then dumps one database.',
	scripts: [script('check', 'before'), script('dump', 'before')]
};

const OWN = {
	id: 'jt_1',
	builtin: false,
	name: 'Web application',
	description: 'Stops the service.',
	scripts: [script('stop', 'before'), script('start', 'finally')]
};

beforeEach(() => {
	route.url = new URL('http://localhost/settings/scripts/jobs');
	session.role = 'admin';
	for (const mock of Object.values(mocks)) mock.mockReset();
	mocks.listJobTemplates.mockResolvedValue([BUILTIN, OWN]);
	mocks.listScriptTemplates.mockResolvedValue([]);
	mocks.listSecrets.mockResolvedValue([]);
	mocks.createJobTemplate.mockResolvedValue(OWN);
	mocks.updateJobTemplate.mockResolvedValue(OWN);
	mocks.deleteJobTemplate.mockResolvedValue(undefined);
});

describe('Job templates screen', () => {
	it('lists the templates with the number of their scripts', async () => {
		const user = userEvent.setup();
		render(Templates);
		// The own templates open first; the built-in ones are on their tab.
		expect(await screen.findByText('Web application')).toBeInTheDocument();
		expect(screen.getByText('2 scripts: before, finally')).toBeInTheDocument();
		expect(screen.queryByText('PostgreSQL server (Linux)')).toBeNull();
		await user.click(screen.getByRole('button', { name: 'Built-in 1' }));
		expect(await screen.findByText('PostgreSQL server (Linux)')).toBeInTheDocument();
		expect(screen.getByText('built-in')).toBeInTheDocument();
	});

	it('links each template to its page and opens a copy as a new template', async () => {
		const user = userEvent.setup();
		render(Templates);
		expect(await screen.findByRole('link', { name: 'Web application' })).toHaveAttribute(
			'href',
			'/settings/scripts/jobs/jt_1'
		);
		await user.click(screen.getByRole('button', { name: 'Built-in 1' }));
		const row = (await screen.findByText('PostgreSQL server (Linux)')).closest('li') as HTMLElement;
		await user.click(within(row).getByRole('button', { name: 'Make a copy' }));
		expect(mocks.goto).toHaveBeenLastCalledWith(
			'/settings/scripts/jobs/new?from=builtin%3Apostgres-linux'
		);
	});

	it('deletes an own template after a confirmation', async () => {
		const user = userEvent.setup();
		render(Templates);
		await screen.findByRole('link', { name: 'Web application' });
		await user.click(screen.getByRole('button', { name: 'Delete' }));
		const dialog = await screen.findByRole('dialog', { name: 'Delete the template?' });
		await user.click(within(dialog).getByRole('button', { name: 'Delete' }));
		await waitFor(() => expect(mocks.deleteJobTemplate).toHaveBeenCalledWith('jt_1'));
	});

	// The UI hides what the API refuses: only an Admin writes a template (ADR-0037, AC-AU-02).
	it('ac_au_02_shows_the_templates_to_an_operator_without_a_way_to_change_them', async () => {
		session.role = 'operator';
		render(Templates);

		expect(await screen.findByRole('link', { name: 'Web application' })).toHaveAttribute(
			'href',
			'/settings/scripts/jobs/jt_1'
		);
		expect(screen.queryByRole('button', { name: 'Add template' })).toBeNull();
		expect(screen.queryByRole('button', { name: 'Make a copy' })).toBeNull();
		expect(screen.queryByRole('button', { name: 'Delete' })).toBeNull();
	});

	it('ac_au_02_hides_the_create_button_from_a_viewer_when_the_api_refuses_the_list', async () => {
		session.role = 'viewer';
		mocks.listJobTemplates.mockRejectedValue(
			new ApiError({
				status: 403,
				code: 'forbidden',
				title: 'You do not have permission for this operation.'
			})
		);
		render(Templates);

		expect(
			await screen.findByText(/You do not have permission for this operation/)
		).toBeInTheDocument();
		expect(screen.queryByRole('button', { name: 'Add template' })).toBeNull();
	});
});
