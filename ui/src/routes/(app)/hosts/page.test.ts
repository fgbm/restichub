/*
 * The Hosts screen lists the pending registrations and renames a host (FR-ID-01, ADR-0026).
 *
 * The test replaces the session store, the toast store and the service, so it renders the
 * screen alone. The pending list keeps only a token that can still enroll a host, and only
 * an Admin sees a write action.
 */
import { render, screen, waitFor, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiError } from '#lib/api/index.ts';
import Hosts from './+page.svelte';

const mocks = vi.hoisted(() => ({
	hostRows: vi.fn(),
	getAgent: vi.fn(),
	listEnrollmentTokens: vi.fn(),
	getEnrollmentToken: vi.fn(),
	createEnrollmentToken: vi.fn(),
	revokeEnrollmentToken: vi.fn(),
	renameAgent: vi.fn(),
	reenrollAgent: vi.fn(),
	replaceAgent: vi.fn(),
	searchAgents: vi.fn(),
	pushToast: vi.fn()
}));

const session = vi.hoisted(() => ({ role: 'admin' as string }));

const route = vi.hoisted(() => ({ url: new URL('http://localhost/hosts') }));

vi.mock('$app/state', () => ({ page: route }));
vi.mock('$app/navigation', () => ({
	goto: async (address: string) => {
		route.url = new URL(address, 'http://localhost');
	}
}));

vi.mock('#lib/stores/session.svelte.ts', () => ({
	getSession: () => ({
		user: {
			id: 'u_01',
			username: 'admin',
			display_name: 'Anna Petrova',
			role: 'admin',
			disabled: false,
			shared: false
		},
		tenant: { id: 't_01', name: 'acme' },
		role: session.role
	})
}));

vi.mock('#lib/stores/toasts.svelte.ts', () => ({ pushToast: mocks.pushToast }));

const stream = vi.hoisted(() => ({
	listeners: [] as ((event: { type: string; agent_id?: string }) => void)[]
}));

vi.mock('#lib/stores/events.svelte.ts', () => ({
	isStreamConnected: () => false,
	onEvent: (listener: (event: { type: string; agent_id?: string }) => void) => {
		stream.listeners.push(listener);
		return () => {
			stream.listeners = stream.listeners.filter((item) => item !== listener);
		};
	}
}));

vi.mock('#lib/api/index.ts', async (importOriginal) => {
	const actual = await importOriginal<typeof import('#lib/api/index.ts')>();
	return {
		...actual,
		service: {
			...actual.service,
			getAgent: mocks.getAgent,
			listEnrollmentTokens: mocks.listEnrollmentTokens,
			getEnrollmentToken: mocks.getEnrollmentToken,
			createEnrollmentToken: mocks.createEnrollmentToken,
			revokeEnrollmentToken: mocks.revokeEnrollmentToken,
			renameAgent: mocks.renameAgent,
			reenrollAgent: mocks.reenrollAgent,
			replaceAgent: mocks.replaceAgent,
			searchAgents: mocks.searchAgents
		}
	};
});

const PAST = '2026-01-01T00:00:00Z';
const FUTURE = '2999-01-01T00:00:00Z';

function token(overrides: Record<string, unknown> = {}) {
	return {
		id: 'e_1',
		display_prefix: 'ab12cd34',
		labels: [],
		expires_at: FUTURE,
		used_at: null,
		revoked_at: null,
		revoked_by_user_id: null,
		created_at: '2026-10-13T00:00:00Z',
		created_by: 'u_01',
		replaces_agent_id: null,
		used_by_agent_id: null,
		...overrides
	};
}

function agent(overrides: Record<string, unknown> = {}) {
	return {
		id: 'a_01',
		hostname: 'win-fs01',
		os: 'windows',
		arch: 'amd64',
		agent_version: '0.4.2',
		restic_version: '0.17.0',
		restore_roots: [],
		labels: [],
		certificate_serial: '0f01',
		certificate_expires_at: FUTURE,
		last_certificate_rotation_at: null,
		superseded_certificate_used_at: null,
		protocol_version: '1.1',
		control_plane_version: '0.4.2',
		version_status: 'current',
		status: 'active',
		online: true,
		last_seen_at: PAST,
		first_connected_at: PAST,
		possible_duplicate_of: null,
		created_at: PAST,
		revoked_at: null,
		revoked_by_user_id: null,
		server_scripts: 'disabled',
		script_interpreters: [],
		...overrides
	};
}

describe('the Hosts screen', () => {
	beforeEach(() => {
		vi.clearAllMocks();
		route.url = new URL('http://localhost/hosts');
		session.role = 'admin';
		mocks.hostRows.mockResolvedValue([]);
		mocks.getAgent.mockRejectedValue(new Error('not found'));
		mocks.listEnrollmentTokens.mockResolvedValue([]);
		// The server filters the page; each read takes the next rows that the test gives.
		mocks.searchAgents.mockImplementation(async () => {
			const items = await mocks.hostRows();
			return {
				items,
				next_cursor: null,
				total: items.length,
				facets: {
					states: { online: 1, offline: 2, waiting: 0, revoked: 3 },
					os: [{ value: 'linux', count: 3 }],
					agent_versions: [],
					labels: []
				}
			};
		});
	});

	it('fr_ui_01 reads one page of the hosts from the server without the revoked hosts', async () => {
		const user = userEvent.setup();
		mocks.hostRows.mockResolvedValue([agent({ id: 'a_01', hostname: 'win-fs01' })]);
		render(Hosts);

		await screen.findByText('win-fs01');
		expect(mocks.searchAgents).toHaveBeenCalledWith(
			expect.objectContaining({
				state: 'online,offline,waiting',
				offset: 0,
				limit: 25,
				facets: true
			})
		);
		expect(screen.getByText('Showing 1 to 1 of 1')).toBeInTheDocument();
		// The tabs count the hosts on the server, also the hidden revoked ones.
		expect(screen.getByRole('button', { name: 'Active 3' })).toHaveAttribute(
			'aria-pressed',
			'true'
		);
		expect(screen.getByRole('button', { name: 'Offline 2' })).toBeInTheDocument();
		await user.click(screen.getByRole('button', { name: 'Revoked 3' }));
		await waitFor(() =>
			expect(mocks.searchAgents).toHaveBeenLastCalledWith(
				expect.objectContaining({ state: 'revoked' })
			)
		);
	});

	it('fr_ui_01 asks for the facets only when a filter changes, not for a new page', async () => {
		const user = userEvent.setup();
		mocks.searchAgents.mockImplementation(async (query: { facets?: boolean }) => ({
			items: [agent()],
			next_cursor: null,
			total: 30,
			total_capped: false,
			facets: query.facets
				? {
						states: { online: 30, offline: 0, waiting: 0, revoked: 0 },
						os: [],
						agent_versions: [],
						labels: []
					}
				: null
		}));
		render(Hosts);

		await screen.findByText('win-fs01');
		expect(mocks.searchAgents).toHaveBeenCalledTimes(1);
		expect(mocks.searchAgents).toHaveBeenLastCalledWith(expect.objectContaining({ facets: true }));
		await user.click(screen.getByRole('button', { name: 'Next' }));
		await waitFor(() =>
			expect(mocks.searchAgents).toHaveBeenLastCalledWith(expect.objectContaining({ offset: 25 }))
		);
		expect(mocks.searchAgents.mock.lastCall?.[0].facets).toBeUndefined();

		await user.type(screen.getByRole('searchbox'), 'web');
		await waitFor(() =>
			expect(mocks.searchAgents).toHaveBeenLastCalledWith(
				expect.objectContaining({ q: 'web', offset: 0, facets: true })
			)
		);
	});

	it('fr_ui_01 chooses the page size and keeps it when Reset clears the filters', async () => {
		const user = userEvent.setup();
		route.url = new URL('http://localhost/hosts?os=windows&size=50');
		mocks.searchAgents.mockImplementation(async () => ({
			items: [agent()],
			next_cursor: null,
			total: 60,
			total_capped: false,
			facets: null
		}));
		render(Hosts);

		await screen.findByText('win-fs01');
		expect(mocks.searchAgents).toHaveBeenLastCalledWith(
			expect.objectContaining({ os: 'windows', offset: 0, limit: 50 })
		);
		expect(screen.getByLabelText('Rows per page')).toHaveValue('50');

		await user.click(screen.getByRole('button', { name: 'Reset' }));
		await waitFor(() =>
			expect(mocks.searchAgents).toHaveBeenLastCalledWith(
				expect.objectContaining({ os: undefined, limit: 50 })
			)
		);
		expect(route.url.searchParams.get('size')).toBe('50');
		expect(route.url.searchParams.get('os')).toBeNull();
	});

	it('fr_ui_01 shows the capped total and asks to narrow the filters', async () => {
		mocks.searchAgents.mockResolvedValue({
			items: [agent()],
			next_cursor: null,
			total: 10_000,
			total_capped: true,
			facets: null
		});
		render(Hosts);

		expect(
			await screen.findByText('More than 10,000 records match. Narrow the filters to see the rest.')
		).toBeInTheDocument();
		expect(screen.getByText('10,000+')).toBeInTheDocument();
	});

	it('shows the name of the host that a token re-registers from another page', async () => {
		mocks.listEnrollmentTokens.mockResolvedValue([token({ replaces_agent_id: 'a_09' })]);
		mocks.getAgent.mockResolvedValue(agent({ id: 'a_09', hostname: 'far-away' }));
		render(Hosts);

		expect(await screen.findByText('Re-registers the host "far-away"')).toBeInTheDocument();
		expect(mocks.getAgent).toHaveBeenCalledWith('a_09');
	});

	it('shows only a registration that can still enroll a host', async () => {
		mocks.listEnrollmentTokens.mockResolvedValue([
			token({ id: 'e_1', display_prefix: 'ab12cd34', labels: ['web'] }),
			token({ id: 'e_2', display_prefix: 'used0000', used_at: PAST }),
			token({ id: 'e_3', display_prefix: 'revoke00', revoked_at: PAST }),
			token({ id: 'e_4', display_prefix: 'expire00', expires_at: PAST })
		]);
		render(Hosts);

		await waitFor(() => expect(screen.getByText('Pending registrations')).toBeInTheDocument());
		expect(screen.getByText('ab12cd34…')).toBeInTheDocument();
		expect(screen.getByText('labels: web')).toBeInTheDocument();
		expect(screen.queryByText('used0000')).not.toBeInTheDocument();
		expect(screen.queryByText('revoke00')).not.toBeInTheDocument();
		expect(screen.queryByText('expire00')).not.toBeInTheDocument();
	});

	it('cancels a pending registration and the row disappears', async () => {
		const user = userEvent.setup();
		mocks.listEnrollmentTokens.mockResolvedValueOnce([token()]).mockResolvedValueOnce([]);
		render(Hosts);

		await screen.findByText('Pending registrations');
		await user.click(screen.getByRole('button', { name: 'Cancel the token ab12cd34' }));
		const dialog = await screen.findByRole('dialog');
		await user.click(within(dialog).getByRole('button', { name: 'Cancel' }));

		await waitFor(() => expect(mocks.revokeEnrollmentToken).toHaveBeenCalledWith('e_1'));
		await waitFor(() =>
			expect(screen.queryByText('Pending registrations')).not.toBeInTheDocument()
		);
	});

	it('hides every write action from a Viewer', async () => {
		session.role = 'viewer';
		mocks.hostRows.mockResolvedValue([agent()]);
		mocks.listEnrollmentTokens.mockResolvedValue([token()]);
		render(Hosts);

		await screen.findByText('ab12cd34…');
		expect(screen.queryByRole('button', { name: 'Add host' })).not.toBeInTheDocument();
		expect(screen.queryByRole('button', { name: /^Cancel the token/ })).not.toBeInTheDocument();
		expect(
			screen.queryByRole('button', { name: 'More actions for win-fs01' })
		).not.toBeInTheDocument();
	});

	it('renames a host and refuses an empty name', async () => {
		const user = userEvent.setup();
		mocks.hostRows.mockResolvedValue([agent()]);
		mocks.renameAgent.mockResolvedValue(agent({ hostname: 'new-name' }));
		render(Hosts);

		await screen.findByText('win-fs01');
		await user.click(screen.getByRole('button', { name: 'More actions for win-fs01' }));
		// The menu content of bits-ui is `visibility: hidden` in jsdom, so a role query cannot see
		// the item. The text query ignores the visibility, as the component test does.
		await user.click(await screen.findByText('Rename'));

		const dialog = await screen.findByRole('dialog');
		const input = document.querySelector<HTMLInputElement>('#host-rename') as HTMLInputElement;
		expect(input.value).toBe('win-fs01');

		await user.clear(input);
		await user.type(input, '   ');
		await user.click(within(dialog).getByRole('button', { name: 'Rename' }));
		expect(await within(dialog).findByText('This field is required.')).toBeVisible();
		expect(input).toHaveFocus();
		expect(mocks.renameAgent).not.toHaveBeenCalled();

		await user.clear(input);
		await user.type(input, '  new-name  ');
		await user.click(within(dialog).getByRole('button', { name: 'Rename' }));

		await waitFor(() => expect(mocks.renameAgent).toHaveBeenCalledWith('a_01', 'new-name'));
	});

	it('shows one link per host and the contact, the system and the agent in line 2', async () => {
		mocks.hostRows.mockResolvedValue([
			agent({ id: 'a_01', hostname: 'win-fs01', labels: ['office', '1c', 'prod'] }),
			agent({ id: 'a_02', hostname: 'srv-db-01', os: 'linux' })
		]);
		render(Hosts);

		await screen.findByText('win-fs01');
		// The item holds one link: the host page (design/style/ui-principles.md, 2.3).
		expect(screen.getAllByRole('link').map((link) => link.getAttribute('href'))).toEqual([
			expect.stringMatching(/\/hosts\/a_01$/),
			expect.stringMatching(/\/hosts\/a_02$/)
		]);
		expect(screen.getAllByText(/^contact .* ago$/)).toHaveLength(2);
		expect(screen.getByText('windows · amd64')).toBeInTheDocument();
		expect(screen.getAllByText('agent 0.4.2')).toHaveLength(2);
		expect(screen.getByText('office, 1c +1')).toBeInTheDocument();
		// A host in the usual state has no pill.
		expect(document.querySelector('.status')).toBeNull();
	});

	it('links the host name to the host page', async () => {
		mocks.hostRows.mockResolvedValue([agent({ id: 'a_01', hostname: 'win-fs01' })]);
		render(Hosts);

		const link = await screen.findByRole('link', { name: 'win-fs01' });
		expect(link).toHaveAttribute('href', expect.stringMatching(/\/hosts\/a_01$/));
	});

	it('marks an agent that needs an update and an agent with an unsupported version (FR-PR-09)', async () => {
		mocks.hostRows.mockResolvedValue([
			agent({ id: 'a_01', hostname: 'host-current' }),
			agent({
				id: 'a_02',
				hostname: 'host-old',
				agent_version: '0.3.0',
				version_status: 'update_available'
			}),
			agent({
				id: 'a_03',
				hostname: 'host-refused',
				agent_version: '0.1.0',
				protocol_version: '0.9',
				version_status: 'unsupported'
			}),
			agent({
				id: 'a_04',
				hostname: 'host-gone',
				version_status: 'update_available',
				status: 'revoked',
				revoked_at: PAST
			})
		]);
		render(Hosts);

		await screen.findByText('host-refused');
		expect(screen.getAllByText('agent 0.3.0, 0.4.2 is available')).toHaveLength(1);
		expect(screen.getByText('Agent too old', { selector: '.status' })).toBeInTheDocument();
		expect(screen.getByText('Revoked', { selector: '.status' })).toBeInTheDocument();
	});

	it('warns before the certificate expiry with one pill (FR-ID-03)', async () => {
		const soon = new Date(Date.now() + 2 * 24 * 60 * 60 * 1000).toISOString();
		mocks.hostRows.mockResolvedValue([
			agent({ id: 'a_01', hostname: 'host-fresh' }),
			agent({ id: 'a_02', hostname: 'host-soon', certificate_expires_at: soon }),
			agent({ id: 'a_03', hostname: 'host-dead', certificate_expires_at: PAST })
		]);
		render(Hosts);

		await screen.findByText('host-soon');
		expect(
			screen.getByText('Certificate expires soon', { selector: '.status' })
		).toBeInTheDocument();
		expect(screen.getByText('Certificate expired', { selector: '.status' })).toBeInTheDocument();
		expect(document.querySelectorAll('.status')).toHaveLength(2);
	});

	it('fr_ui_01 marks a host whose old certificate connected with the danger pill', async () => {
		mocks.hostRows.mockResolvedValue([
			agent({ id: 'a_01', hostname: 'host-fresh' }),
			agent({
				id: 'a_02',
				hostname: 'host-copied',
				online: false,
				superseded_certificate_used_at: PAST
			})
		]);
		const { container } = render(Hosts);

		await screen.findByText('host-copied');
		const pill = screen.getByText('Old certificate used', { selector: '.status' });
		expect(pill).toHaveClass('status--failed');
		expect(container.querySelectorAll('.status')).toHaveLength(1);
		const item = pill.closest('li') as HTMLElement;
		expect(item.querySelector('.item__lead')?.className).toMatch(/tone--danger/);
	});

	it('fr_ui_02 a host event reads the list again and shows the host offline', async () => {
		mocks.hostRows
			.mockResolvedValueOnce([agent({ id: 'a_01', hostname: 'host-stopped' })])
			.mockResolvedValue([agent({ id: 'a_01', hostname: 'host-stopped', online: false })]);
		render(Hosts);

		await screen.findByText('host-stopped');
		expect(screen.queryByText('Offline', { selector: '.status' })).not.toBeInTheDocument();
		for (const listener of stream.listeners) listener({ type: 'host.offline', agent_id: 'a_01' });

		expect(await screen.findByText('Offline', { selector: '.status' })).toBeInTheDocument();
	});

	it('shows a host that never connected as waiting (FR-ID-04)', async () => {
		mocks.hostRows.mockResolvedValue([
			agent({ id: 'a_01', hostname: 'host-live' }),
			agent({ id: 'a_02', hostname: 'host-new', first_connected_at: null })
		]);
		render(Hosts);

		await screen.findByText('host-new');
		expect(screen.getByText('Waiting for connection', { selector: '.status' })).toBeInTheDocument();
		expect(screen.getByText(/^registered .* ago$/)).toBeInTheDocument();
	});

	it('offers the replacement only on the newer record (FR-ID-11, AC-ID-09)', async () => {
		const user = userEvent.setup({ pointerEventsCheck: 0 });
		mocks.hostRows.mockResolvedValue([
			agent({
				id: 'a_01',
				hostname: 'old-host',
				possible_duplicate_of: {
					id: 'a_02',
					hostname: 'new-host',
					created_at: PAST,
					last_seen_at: PAST,
					is_older: false
				}
			}),
			agent({
				id: 'a_02',
				hostname: 'new-host',
				possible_duplicate_of: {
					id: 'a_01',
					hostname: 'old-host',
					created_at: PAST,
					last_seen_at: PAST,
					is_older: true
				}
			})
		]);
		mocks.replaceAgent.mockResolvedValue({
			plans_moved: 2,
			repositories_moved: 0,
			queued_jobs_moved: 0
		});
		render(Hosts);

		await screen.findByText('new-host');
		// AC-ID-09: both records of a duplicate show the warning in the list.
		expect(screen.getAllByText('Possible duplicate')).toHaveLength(2);
		await user.click(screen.getByRole('button', { name: 'More actions for old-host' }));
		await screen.findByText('Re-register');
		expect(screen.queryByText('Replace the old record')).not.toBeInTheDocument();
		await user.keyboard('{Escape}');

		await user.click(screen.getByRole('button', { name: 'More actions for new-host' }));
		await user.click(await screen.findByText('Replace the old record'));
		const dialog = await screen.findByRole('dialog');
		expect(within(dialog).getByText(/Last contact/)).toBeInTheDocument();
		await user.click(within(dialog).getByRole('button', { name: 'Replace' }));

		await waitFor(() => expect(mocks.replaceAgent).toHaveBeenCalledWith('a_02', 'a_01'));
	});

	it('issues a re-registration token and shows the command (FR-ID-01)', async () => {
		const user = userEvent.setup();
		mocks.hostRows.mockResolvedValue([agent()]);
		mocks.reenrollAgent.mockResolvedValue({
			id: 'e_9',
			token: 're_SECRETVALUE',
			display_prefix: 'SECRETVA',
			labels: [],
			expires_at: FUTURE,
			created_at: PAST,
			replaces_agent_id: 'a_01'
		});
		render(Hosts);

		await screen.findByText('win-fs01');
		await user.click(screen.getByRole('button', { name: 'More actions for win-fs01' }));
		await user.click(await screen.findByText('Re-register'));
		const confirm = await screen.findByRole('dialog');
		await user.click(within(confirm).getByRole('button', { name: 'Create token' }));

		await waitFor(() => expect(mocks.reenrollAgent).toHaveBeenCalledWith('a_01'));
		const dialog = await screen.findByRole('dialog', { name: 'Copy the re-registration token' });
		expect(
			within(dialog).getByDisplayValue(/--force .*--token re_SECRETVALUE/)
		).toBeInTheDocument();
	});

	it('refreshes the list after a conflict on cancel (FR-ID-01)', async () => {
		const user = userEvent.setup();
		mocks.listEnrollmentTokens.mockResolvedValueOnce([token()]).mockResolvedValue([]);
		mocks.revokeEnrollmentToken.mockRejectedValue(
			new ApiError({ status: 409, code: 'token_used', title: 'Conflict' })
		);
		render(Hosts);

		await screen.findByText('Pending registrations');
		await user.click(screen.getByRole('button', { name: 'Cancel the token ab12cd34' }));
		const dialog = await screen.findByRole('dialog');
		await user.click(within(dialog).getByRole('button', { name: 'Cancel' }));

		await waitFor(() =>
			expect(mocks.pushToast).toHaveBeenCalledWith(
				'danger',
				'A host already used this enrollment token. You cannot cancel it. Create a new token if you need one.'
			)
		);
		await waitFor(() =>
			expect(screen.queryByText('Pending registrations')).not.toBeInTheDocument()
		);
	});

	it('fr_id_01 sends the labels of the Add host dialog as an array', async () => {
		const user = userEvent.setup();
		mocks.createEnrollmentToken.mockResolvedValue({
			id: 'e_1',
			token: 're_SECRETVALUE',
			display_prefix: 'SECRETVA',
			labels: ['test', 'timeweb'],
			expires_at: FUTURE,
			created_at: PAST,
			replaces_agent_id: null
		});
		render(Hosts);

		await user.click(await screen.findByRole('button', { name: 'Add host' }));
		const dialog = await screen.findByRole('dialog');
		const field = within(dialog).getByLabelText('Labels');
		await user.type(field, 'test, timeweb, Bad_label,');
		expect(within(dialog).getByRole('alert')).toHaveTextContent(
			'Bad_label: Use only the letters a-z, the digits 0-9 and the hyphen.'
		);
		expect(field).toHaveValue('Bad_label,');
		await user.clear(field);
		await user.click(within(dialog).getByRole('button', { name: 'Remove test' }));
		await user.type(field, 'test ');
		await user.click(within(dialog).getByRole('button', { name: 'Create token' }));

		await within(dialog).findByDisplayValue('re_SECRETVALUE');
		expect(mocks.createEnrollmentToken).toHaveBeenCalledWith({ labels: ['timeweb', 'test'] });
	});

	it('fr_id_01 sends no labels when the Add host dialog has none', async () => {
		const user = userEvent.setup();
		mocks.createEnrollmentToken.mockResolvedValue({
			id: 'e_1',
			token: 're_SECRETVALUE',
			display_prefix: 'SECRETVA',
			labels: [],
			expires_at: FUTURE,
			created_at: PAST,
			replaces_agent_id: null
		});
		render(Hosts);

		await user.click(await screen.findByRole('button', { name: 'Add host' }));
		const dialog = await screen.findByRole('dialog');
		await user.click(within(dialog).getByRole('button', { name: 'Create token' }));

		await within(dialog).findByDisplayValue('re_SECRETVALUE');
		expect(mocks.createEnrollmentToken).toHaveBeenCalledWith({ labels: undefined });
	});

	it('tells in the Add host dialog that the host used the token (FR-ID-01)', async () => {
		const user = userEvent.setup();
		vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval'] });
		try {
			mocks.createEnrollmentToken.mockResolvedValue({
				id: 'e_1',
				token: 're_SECRETVALUE',
				display_prefix: 'SECRETVA',
				labels: [],
				expires_at: FUTURE,
				created_at: PAST,
				replaces_agent_id: null
			});
			mocks.listEnrollmentTokens
				.mockResolvedValueOnce([])
				.mockResolvedValueOnce([token()])
				.mockResolvedValue([]);
			mocks.getEnrollmentToken.mockResolvedValue(
				token({ used_at: PAST, used_by_agent_id: 'a_07' })
			);
			mocks.hostRows
				.mockResolvedValueOnce([])
				.mockResolvedValueOnce([])
				.mockResolvedValue([agent({ id: 'a_07', hostname: 'brand-new' })]);
			render(Hosts);

			await user.click(await screen.findByRole('button', { name: 'Add host' }));
			const dialog = await screen.findByRole('dialog');
			await user.click(within(dialog).getByRole('button', { name: 'Create token' }));
			await within(dialog).findByDisplayValue('re_SECRETVALUE');

			await vi.advanceTimersByTimeAsync(10_000);
			await within(dialog).findByText('Host registered: brand-new');
			expect(within(dialog).getByRole('heading', { name: 'Host registered' })).toBeInTheDocument();
			expect(within(dialog).queryByText('Copy the enrollment token')).not.toBeInTheDocument();
			expect(mocks.getEnrollmentToken).toHaveBeenCalledTimes(1);
			expect(mocks.getEnrollmentToken).toHaveBeenCalledWith('e_1');
		} finally {
			vi.useRealTimers();
		}
	});

	describe('the periodic refresh', () => {
		beforeEach(() => {
			vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval'] });
		});
		afterEach(() => {
			vi.useRealTimers();
		});

		it('fr_ui_01 keeps the newest page when an older poll answers late', async () => {
			const user = userEvent.setup();
			let answerPoll: (rows: unknown[]) => void = () => {};
			const page = (rows: unknown[]) => ({
				items: rows,
				next_cursor: null,
				total: 30,
				total_capped: false,
				facets: null
			});
			mocks.searchAgents
				.mockResolvedValueOnce(page([agent({ id: 'a_01', hostname: 'first-page' })]))
				.mockReturnValueOnce(
					new Promise((resolve) => {
						answerPoll = (rows) => resolve(page(rows));
					})
				)
				.mockResolvedValue(page([agent({ id: 'a_02', hostname: 'second-page' })]));
			render(Hosts);

			await screen.findByText('first-page');
			await vi.advanceTimersByTimeAsync(10_000);
			expect(mocks.searchAgents).toHaveBeenCalledTimes(2);
			await user.click(screen.getByRole('button', { name: 'Next' }));
			await screen.findByText('second-page');
			answerPoll([agent({ id: 'a_03', hostname: 'stale-page' })]);
			await new Promise((resolve) => setTimeout(resolve, 0));
			expect(screen.getByText('second-page')).toBeInTheDocument();
			expect(screen.queryByText('stale-page')).not.toBeInTheDocument();
		});

		it('reloads the pending list every 10 seconds (FR-ID-01)', async () => {
			mocks.listEnrollmentTokens.mockResolvedValueOnce([token()]).mockResolvedValue([]);
			render(Hosts);

			await screen.findByText('Pending registrations');
			await vi.advanceTimersByTimeAsync(10_000);
			await waitFor(() =>
				expect(screen.queryByText('Pending registrations')).not.toBeInTheDocument()
			);
			expect(mocks.listEnrollmentTokens).toHaveBeenCalledTimes(2);
			for (const call of mocks.listEnrollmentTokens.mock.calls) {
				expect(call).toEqual([{ pending: true }]);
			}
			expect(mocks.getEnrollmentToken).not.toHaveBeenCalled();
		});
	});
});
