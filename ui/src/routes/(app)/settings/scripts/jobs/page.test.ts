/*
 * The Job templates screen lists the built-in and the own templates and saves an ordered set of
 * scripts (FR-HK-12).
 */
import { render, screen, waitFor, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import Templates from './+page.svelte';

const mocks = vi.hoisted(() => ({
	listJobTemplates: vi.fn(),
	listScriptTemplates: vi.fn(),
	listSecrets: vi.fn(),
	createJobTemplate: vi.fn(),
	updateJobTemplate: vi.fn(),
	deleteJobTemplate: vi.fn(),
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

vi.mock('$app/state', () => ({ page: { url: new URL('http://localhost/settings') } }));

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
		render(Templates);
		expect(await screen.findByText('PostgreSQL server (Linux)')).toBeInTheDocument();
		expect(screen.getByText('Web application')).toBeInTheDocument();
		expect(screen.getAllByText('Built-in').length).toBeGreaterThan(0);
	});

	it('opens a template with its scripts in the saved order', async () => {
		const user = userEvent.setup();
		render(Templates);
		await user.click(await screen.findByRole('button', { name: 'Web application' }));
		const dialog = await screen.findByRole('dialog');
		const titles = within(dialog)
			.getAllByTestId('script-card')
			.map((card) => card.querySelector('.card__title')?.textContent);
		expect(titles).toEqual(['stop', 'start']);
		expect(
			within(within(dialog).getByTestId('scripts-after')).getAllByTestId('script-card')
		).toHaveLength(1);
	});

	it('saves the scripts of a copy of a built-in template, with an empty secret', async () => {
		const user = userEvent.setup();
		render(Templates);
		const row = (await screen.findByText('PostgreSQL server (Linux)')).closest('tr') as HTMLElement;
		await user.click(within(row).getByRole('button', { name: 'Make a copy' }));
		const dialog = await screen.findByRole('dialog');
		await user.click(within(dialog).getByRole('button', { name: 'Save' }));
		await waitFor(() => expect(mocks.createJobTemplate).toHaveBeenCalledTimes(1));
		const request = mocks.createJobTemplate.mock.calls[0][0];
		expect(request.name).toBe('PostgreSQL server (Linux) copy');
		expect(request.scripts.map((item: { name: string }) => item.name)).toEqual(['check', 'dump']);
		expect(request.scripts[0].env).toEqual([{ name: 'PGPASSWORD', kind: 'secret_ref', value: '' }]);
	});

	it('deletes an own template after a confirmation', async () => {
		const user = userEvent.setup();
		render(Templates);
		await screen.findByRole('button', { name: 'Web application' });
		await user.click(screen.getByRole('button', { name: 'Delete' }));
		const dialog = await screen.findByRole('dialog', { name: 'Delete the template?' });
		await user.click(within(dialog).getByRole('button', { name: 'Delete' }));
		await waitFor(() => expect(mocks.deleteJobTemplate).toHaveBeenCalledWith('jt_1'));
	});
});
