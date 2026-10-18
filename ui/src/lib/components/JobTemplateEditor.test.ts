/*
 * The job template page shows the scripts in their order, saves a copy of a built-in template and
 * deletes an own template (FR-HK-12).
 */
import { render, screen, waitFor, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import JobTemplateEditor from './JobTemplateEditor.svelte';

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

const session = vi.hoisted(() => ({ role: 'admin' as string }));

vi.mock('#lib/stores/session.svelte.ts', () => ({
	getSession: () => ({
		user: { id: 'u_01', username: 'admin', display_name: 'Anna', role: session.role },
		tenant: { id: 't_01', name: 'acme' },
		role: session.role
	})
}));

vi.mock('#lib/stores/toasts.svelte.ts', () => ({ pushToast: mocks.pushToast }));

vi.mock('$app/state', () => ({ page: { url: new URL('http://localhost/settings/scripts/jobs') } }));
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
	session.role = 'admin';
	for (const mock of Object.values(mocks)) mock.mockReset();
	mocks.listJobTemplates.mockResolvedValue([BUILTIN, OWN]);
	mocks.listScriptTemplates.mockResolvedValue([]);
	mocks.listSecrets.mockResolvedValue([]);
	mocks.createJobTemplate.mockResolvedValue(OWN);
	mocks.updateJobTemplate.mockResolvedValue(OWN);
	mocks.deleteJobTemplate.mockResolvedValue(undefined);
});

describe('Job template page', () => {
	it('shows the scripts in their saved order and the run order panel', async () => {
		render(JobTemplateEditor, { templateId: 'jt_1' });
		await screen.findByRole('heading', { level: 1, name: 'Web application' });
		const titles = screen
			.getAllByTestId('script-card')
			.map((card) => card.querySelector('.card__title')?.textContent);
		expect(titles).toEqual(['stop', 'start']);
		expect(within(screen.getByTestId('scripts-after')).getAllByTestId('script-card')).toHaveLength(
			1
		);
		const order = screen
			.getByRole('heading', { name: 'Run order' })
			.closest('section') as HTMLElement;
		expect(
			[...order.querySelectorAll('li')].map((item) => item.textContent?.replace(/\s+/g, ' ').trim())
		).toEqual(['Before stop', 'Backup', 'Finally start']);
	});

	it('shows a built-in template read-only with a copy action', async () => {
		const user = userEvent.setup();
		render(JobTemplateEditor, { templateId: 'builtin:postgres-linux' });
		await screen.findByRole('heading', { level: 1, name: 'PostgreSQL server (Linux)' });
		expect(
			screen.getByText('A built-in template is read-only. Make a copy to change it.')
		).toBeInTheDocument();
		expect(screen.queryByRole('button', { name: 'Save' })).toBeNull();
		await user.click(screen.getByRole('button', { name: 'Make a copy' }));
		expect(mocks.goto).toHaveBeenCalledWith(
			'/settings/scripts/jobs/new?from=builtin%3Apostgres-linux'
		);
	});

	it('saves the scripts of a copy of a built-in template, with an empty secret', async () => {
		const user = userEvent.setup();
		render(JobTemplateEditor, { copyFrom: 'builtin:postgres-linux' });
		await screen.findByRole('heading', { level: 1, name: 'New job template' });
		await user.click(screen.getByRole('button', { name: 'Save' }));
		await waitFor(() => expect(mocks.createJobTemplate).toHaveBeenCalledTimes(1));
		const request = mocks.createJobTemplate.mock.calls[0][0];
		expect(request.name).toBe('PostgreSQL server (Linux) copy');
		expect(request.scripts.map((item: { name: string }) => item.name)).toEqual(['check', 'dump']);
		expect(request.scripts[0].env).toEqual([{ name: 'PGPASSWORD', kind: 'secret_ref', value: '' }]);
		expect(mocks.goto).toHaveBeenCalledWith('/settings/scripts/jobs');
	});

	it('keeps the delete of an own template in the dangerous section only', async () => {
		render(JobTemplateEditor, { templateId: 'jt_1' });
		await screen.findByRole('heading', { level: 1, name: 'Web application' });

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
		const builtin = render(JobTemplateEditor, { templateId: 'builtin:postgres-linux' });
		await screen.findByRole('heading', { level: 1, name: 'PostgreSQL server (Linux)' });
		expect(document.querySelector('[data-section="danger"]')).toBeNull();
		builtin.unmount();

		const created = render(JobTemplateEditor);
		await screen.findByRole('heading', { level: 1, name: 'New job template' });
		expect(document.querySelector('[data-section="danger"]')).toBeNull();
		created.unmount();

		session.role = 'viewer';
		render(JobTemplateEditor, { templateId: 'jt_1' });
		await screen.findByRole('heading', { level: 1, name: 'Web application' });
		expect(document.querySelector('[data-section="danger"]')).toBeNull();
	});

	it('deletes an own template after a confirmation and returns to the list', async () => {
		const user = userEvent.setup();
		render(JobTemplateEditor, { templateId: 'jt_1' });
		await screen.findByRole('heading', { level: 1, name: 'Web application' });
		await user.click(screen.getByRole('button', { name: 'Delete' }));
		const dialog = await screen.findByRole('dialog', { name: 'Delete the template?' });
		await user.click(within(dialog).getByRole('button', { name: 'Delete' }));
		await waitFor(() => expect(mocks.deleteJobTemplate).toHaveBeenCalledWith('jt_1'));
		expect(mocks.goto).toHaveBeenCalledWith('/settings/scripts/jobs');
	});
});
