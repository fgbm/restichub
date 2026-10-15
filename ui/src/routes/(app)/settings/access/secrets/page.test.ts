/*
 * The Secrets screen lists the metadata with the users of each secret, and it never reads or shows
 * a value (FR-RP-05, AC-RP-02).
 */
import { render, screen, waitFor, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import Secrets from './+page.svelte';

const mocks = vi.hoisted(() => ({
	listSecrets: vi.fn(),
	createSecret: vi.fn(),
	replaceSecretValue: vi.fn(),
	deleteSecret: vi.fn(),
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

vi.mock('$app/state', () => ({
	page: { url: new URL('http://localhost/settings/access/secrets') }
}));

vi.mock('#lib/api/index.ts', async (importOriginal) => {
	const actual = await importOriginal<typeof import('#lib/api/index.ts')>();
	return {
		...actual,
		service: {
			...actual.service,
			listSecrets: mocks.listSecrets,
			createSecret: mocks.createSecret,
			replaceSecretValue: mocks.replaceSecretValue,
			deleteSecret: mocks.deleteSecret
		}
	};
});

const BASE = {
	created_at: '2026-10-01T10:00:00Z',
	updated_at: '2026-10-01T10:00:00Z',
	created_by: 'u_01'
};

const FREE = { ...BASE, id: 's_free', name: 'spare.token', purpose: 'hook_secret', used_by: [] };
const SCRIPT = {
	...BASE,
	id: 's_db',
	name: 'db.password',
	purpose: 'hook_secret',
	used_by: [{ kind: 'profile', id: 'p_3', name: 'db-nightly' }]
};
const REPO = {
	...BASE,
	id: 's_repo',
	name: 'primary-s3.password',
	purpose: 'repository_password',
	used_by: [{ kind: 'repository', id: 'r_1', name: 'primary-s3' }]
};

beforeEach(() => {
	for (const mock of Object.values(mocks)) mock.mockReset();
	mocks.listSecrets.mockResolvedValue([FREE, SCRIPT, REPO]);
	mocks.createSecret.mockResolvedValue(FREE);
	mocks.replaceSecretValue.mockResolvedValue(undefined);
	mocks.deleteSecret.mockResolvedValue(undefined);
});

describe('Secrets screen', () => {
	it('lists the secrets with their purpose and their users as links', async () => {
		render(Secrets);
		expect(await screen.findByText('db.password')).toBeInTheDocument();
		expect(screen.getAllByText('Script secret').length).toBeGreaterThan(0);
		expect(screen.getByText('Repository password')).toBeInTheDocument();
		expect(screen.getByRole('link', { name: 'db-nightly' })).toHaveAttribute('href', '/profiles');
		expect(screen.getByRole('link', { name: 'primary-s3' })).toHaveAttribute(
			'href',
			'/repositories'
		);
		expect(screen.getByText('Not used')).toBeInTheDocument();
	});

	it('hides the replace action for the password of a repository', async () => {
		render(Secrets);
		await screen.findByText('db.password');
		expect(screen.getAllByRole('button', { name: 'Replace value' })).toHaveLength(2);
	});

	it('creates a secret and sends the value once', async () => {
		const user = userEvent.setup();
		render(Secrets);
		await screen.findByText('db.password');
		await user.click(screen.getByRole('button', { name: 'Add secret' }));
		const dialog = await screen.findByRole('dialog');
		await user.type(within(dialog).getByLabelText(/^Name/), 'new.token');
		const value = within(dialog).getByLabelText(/^Value/);
		expect(value).toHaveAttribute('type', 'password');
		await user.type(value, 'canary-value');
		await user.click(within(dialog).getByRole('button', { name: 'Add secret' }));
		await waitFor(() =>
			expect(mocks.createSecret).toHaveBeenCalledWith({
				name: 'new.token',
				purpose: 'hook_secret',
				value: 'canary-value'
			})
		);
		expect(screen.queryByText('canary-value')).not.toBeInTheDocument();
	});

	it('replaces the value and tells that the next delivery uses it', async () => {
		const user = userEvent.setup();
		render(Secrets);
		const row = (await screen.findByText('db.password')).closest('tr') as HTMLElement;
		await user.click(within(row).getByRole('button', { name: 'Replace value' }));
		const dialog = await screen.findByRole('dialog');
		expect(within(dialog).getByText(/next delivery/)).toBeInTheDocument();
		await user.type(within(dialog).getByLabelText(/^New value/), 'fresh');
		await user.click(within(dialog).getByRole('button', { name: 'Replace value' }));
		await waitFor(() => expect(mocks.replaceSecretValue).toHaveBeenCalledWith('s_db', 'fresh'));
	});

	it('deletes a free secret after the confirmation', async () => {
		const user = userEvent.setup();
		render(Secrets);
		const row = (await screen.findByText('spare.token')).closest('tr') as HTMLElement;
		await user.click(within(row).getByRole('button', { name: 'Delete' }));
		const dialog = await screen.findByRole('dialog');
		await user.click(within(dialog).getByRole('button', { name: 'Delete' }));
		await waitFor(() => expect(mocks.deleteSecret).toHaveBeenCalledWith('s_free'));
	});

	it('refuses to delete a secret in use and names its users', async () => {
		const user = userEvent.setup();
		render(Secrets);
		const row = (await screen.findByText('db.password')).closest('tr') as HTMLElement;
		await user.click(within(row).getByRole('button', { name: 'Delete' }));
		const dialog = await screen.findByRole('dialog');
		expect(within(dialog).getByText('The secret is in use')).toBeInTheDocument();
		expect(within(dialog).getByRole('link', { name: 'db-nightly' })).toBeInTheDocument();
		expect(within(dialog).getByRole('button', { name: 'Delete' })).toBeDisabled();
		expect(mocks.deleteSecret).not.toHaveBeenCalled();
	});
});
