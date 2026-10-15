/*
 * The channel settings create an email channel and a webhook, show the signing secret one time
 * only, test a channel and delete it (FR-MN-04, AC-MN-06, AC-MN-08).
 */
import { render, screen, waitFor, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiError } from '#lib/api/index.ts';
import Channels from './+page.svelte';

const mocks = vi.hoisted(() => ({
	listChannels: vi.fn(),
	createChannel: vi.fn(),
	updateChannel: vi.fn(),
	deleteChannel: vi.fn(),
	testChannel: vi.fn(),
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

vi.mock('#lib/api/index.ts', async (importOriginal) => {
	const actual = await importOriginal<typeof import('#lib/api/index.ts')>();
	return {
		...actual,
		service: {
			...actual.service,
			listChannels: mocks.listChannels,
			createChannel: mocks.createChannel,
			updateChannel: mocks.updateChannel,
			deleteChannel: mocks.deleteChannel,
			testChannel: mocks.testChannel
		}
	};
});

const NOW = '2026-10-15T08:00:00Z';

function channel(overrides: Record<string, unknown> = {}) {
	return {
		id: 'ch_1',
		name: 'ops-mail',
		kind: 'email',
		enabled: true,
		recipients: ['ops@example.com'],
		url_origin: null,
		created_at: NOW,
		updated_at: NOW,
		...overrides
	};
}

function list(items: unknown[], emailConfigured = true) {
	return { items, email_configured: emailConfigured, webhook_allow_http: false };
}

describe('the channel settings', () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	it('lists a channel with its target', async () => {
		mocks.listChannels.mockResolvedValue(list([channel()]));
		render(Channels);

		expect(await screen.findByText('ops-mail')).toBeInTheDocument();
		expect(screen.getByText('ops@example.com')).toBeInTheDocument();
	});

	it('warns when the installation has no SMTP server', async () => {
		mocks.listChannels.mockResolvedValue(list([channel()], false));
		render(Channels);

		expect(await screen.findByText(/no SMTP server/)).toBeInTheDocument();
	});

	it('creates an email channel from the recipient lines', async () => {
		const user = userEvent.setup();
		mocks.listChannels.mockResolvedValueOnce(list([])).mockResolvedValueOnce(list([channel()]));
		mocks.createChannel.mockResolvedValue({ channel: channel(), signing_secret: null });
		render(Channels);

		await screen.findByText('No channels yet');
		await user.click(screen.getAllByRole('button', { name: 'Add channel' })[0]);
		const dialog = await screen.findByRole('dialog');
		await user.type(within(dialog).getByLabelText(/Name/), 'ops-mail');
		await user.type(
			within(dialog).getByLabelText(/Recipients/),
			'ops@example.com\nbackup@example.com'
		);
		await user.click(within(dialog).getByRole('button', { name: 'Save' }));

		await waitFor(() =>
			expect(mocks.createChannel).toHaveBeenCalledWith({
				name: 'ops-mail',
				kind: 'email',
				enabled: true,
				recipients: ['ops@example.com', 'backup@example.com']
			})
		);
		expect(mocks.pushToast).toHaveBeenCalledWith('success', 'The channel is created.');
	});

	it('shows the signing secret of a new webhook one time only', async () => {
		const user = userEvent.setup();
		mocks.listChannels.mockResolvedValue(list([]));
		mocks.createChannel.mockResolvedValue({
			channel: channel({ id: 'ch_2', name: 'hook', kind: 'webhook', recipients: [] }),
			signing_secret: 'whsec_abc123'
		});
		render(Channels);

		await screen.findByText('No channels yet');
		await user.click(screen.getAllByRole('button', { name: 'Add channel' })[0]);
		const dialog = await screen.findByRole('dialog');
		await user.type(within(dialog).getByLabelText(/Name/), 'hook');
		await user.selectOptions(within(dialog).getByLabelText('Type'), 'webhook');
		await user.type(within(dialog).getByLabelText(/Webhook URL/), 'https://hooks.example.com/in/x');
		await user.click(within(dialog).getByRole('button', { name: 'Save' }));

		const secretDialog = await screen.findByRole('dialog', { name: 'Signing secret' });
		expect(within(secretDialog).getByDisplayValue('whsec_abc123')).toBeInTheDocument();
		await user.click(within(secretDialog).getByRole('button', { name: 'Done' }));
		await waitFor(() => expect(screen.queryByDisplayValue('whsec_abc123')).not.toBeInTheDocument());
	});

	it('names the reason when the control plane refuses the URL', async () => {
		const user = userEvent.setup();
		mocks.listChannels.mockResolvedValue(list([]));
		mocks.createChannel.mockRejectedValue(
			new ApiError({
				status: 400,
				code: 'invalid_argument',
				title: 'Bad Request',
				field: 'url',
				reason: 'reserved'
			})
		);
		render(Channels);

		await screen.findByText('No channels yet');
		await user.click(screen.getAllByRole('button', { name: 'Add channel' })[0]);
		const dialog = await screen.findByRole('dialog');
		await user.type(within(dialog).getByLabelText(/Name/), 'hook');
		await user.selectOptions(within(dialog).getByLabelText('Type'), 'webhook');
		await user.type(within(dialog).getByLabelText(/Webhook URL/), 'https://127.0.0.1/x');
		await user.click(within(dialog).getByRole('button', { name: 'Save' }));

		expect(await within(dialog).findByText(/local or private/)).toBeInTheDocument();
	});

	async function openTest(user: ReturnType<typeof userEvent.setup>): Promise<void> {
		mocks.listChannels.mockResolvedValue(
			list([
				channel({ id: 'ch_2', name: 'hook', kind: 'webhook', url_origin: 'https://h.example.com' })
			])
		);
		render(Channels);
		await screen.findByText('hook');
		await user.click(screen.getByRole('button', { name: 'More actions for hook' }));
		await user.click(await screen.findByText('Send test'));
	}

	it('sends a test message', async () => {
		const user = userEvent.setup();
		mocks.testChannel.mockResolvedValue({ ok: true, error: null });
		await openTest(user);

		await waitFor(() => expect(mocks.testChannel).toHaveBeenCalledWith('ch_2'));
		expect(mocks.pushToast).toHaveBeenCalledWith('success', 'The test message is sent.');
	});

	it('reports a failed test message with its code', async () => {
		const user = userEvent.setup();
		mocks.testChannel.mockResolvedValue({ ok: false, error: 'blocked_address' });
		await openTest(user);

		await waitFor(() =>
			expect(mocks.pushToast).toHaveBeenCalledWith(
				'danger',
				'The test message failed: The address is local or private.'
			)
		);
	});

	it('deletes a channel after the confirmation', async () => {
		const user = userEvent.setup();
		mocks.listChannels.mockResolvedValueOnce(list([channel()])).mockResolvedValueOnce(list([]));
		mocks.deleteChannel.mockResolvedValue(undefined);
		render(Channels);
		await screen.findByText('ops-mail');

		await user.click(screen.getByRole('button', { name: 'More actions for ops-mail' }));
		await user.click(await screen.findByText('Delete channel'));
		const dialog = await screen.findByRole('dialog', { name: 'Delete the channel?' });
		await user.click(within(dialog).getByRole('button', { name: 'Delete channel' }));

		await waitFor(() => expect(mocks.deleteChannel).toHaveBeenCalledWith('ch_1'));
		expect(mocks.pushToast).toHaveBeenCalledWith('success', 'The channel is deleted.');
	});
});
