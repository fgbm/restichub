import { render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import RecordPicker from './RecordPicker.svelte';
import type { RecordOption } from './types';

const options: RecordOption[] = [
	{ value: 'p1', label: 'user-profiles', description: ['test-s3-yandex', { tag: 'S3' }] },
	{ value: 'p2', label: 'db-hourly', description: ['rest-main', { tag: 'REST' }] },
	{
		value: 'p3',
		label: 'nas-files',
		description: ['nas-local', { tag: 'Local' }, 'host MINDCORE']
	}
];

// jsdom has no layout, so the floating layer keeps the open list hidden and a role query does
// not find its elements by name.
const field = () => screen.getByRole('combobox', { name: /^Profile/ });
const search = () => screen.getByPlaceholderText('Search');
const option = (name: string) =>
	screen.getByText(name, { selector: '.rh-record__option-label' }).closest('[role="option"]')!;
const options_ = () => screen.queryAllByRole('option', { hidden: true });
const listbox = () => screen.queryByRole('listbox', { hidden: true });

describe('RecordPicker', () => {
	it('shows the label and the first part of the secondary line when closed', () => {
		render(RecordPicker, { props: { label: 'Profile', options, value: 'p1' } });

		expect(field()).toHaveTextContent('user-profiles');
		expect(field()).toHaveTextContent('· test-s3-yandex');
		expect(field()).toHaveAccessibleName('Profile user-profiles');
		expect(field()).toHaveAccessibleDescription('· test-s3-yandex');
	});

	it('finds an option by its secondary text and selects it', async () => {
		const user = userEvent.setup();
		const onchange = vi.fn();
		render(RecordPicker, { props: { label: 'Profile', options, value: 'p1', onchange } });

		await user.click(field());
		await waitFor(() => expect(search()).toHaveFocus());
		expect(search()).toHaveAttribute('role', 'combobox');
		expect(search()).toHaveAttribute('aria-label', 'Find: Profile');
		await user.type(search(), 'NAS-LOC');
		expect(options_()).toHaveLength(1);
		const match = option('nas-files');
		const detail = document.getElementById(match.getAttribute('aria-describedby') ?? '');
		expect(detail).toHaveTextContent(/nas-local\s*Local\s*·\s*host MINDCORE/);

		await user.click(match);
		expect(onchange).toHaveBeenCalledWith('p3');
		await waitFor(() => expect(field()).toHaveTextContent('nas-files'));
		expect(listbox()).not.toBeInTheDocument();
	});

	it('opens and selects with the keyboard and returns the focus to the field', async () => {
		const user = userEvent.setup();
		const onchange = vi.fn();
		render(RecordPicker, { props: { label: 'Profile', options, value: '', onchange } });

		field().focus();
		await user.keyboard('{ArrowDown}');
		await waitFor(() => expect(search()).toHaveFocus());
		await user.keyboard('{ArrowDown}{Enter}');

		expect(onchange).toHaveBeenCalledWith('p2');
		await waitFor(() => expect(field()).toHaveFocus());
		expect(field()).toHaveTextContent('db-hourly');
	});

	it('starts the search with the typed character and closes on Escape', async () => {
		const user = userEvent.setup();
		render(RecordPicker, { props: { label: 'Profile', options, value: 'p1' } });

		field().focus();
		await user.keyboard('r');
		await waitFor(() => expect(search()).toHaveValue('r'));
		expect(options_()).toHaveLength(3);
		await user.type(search(), 'est');
		expect(options_()).toHaveLength(1);

		await user.keyboard('{Escape}');
		await waitFor(() => expect(field()).toHaveFocus());
		expect(field()).toHaveTextContent('user-profiles');
	});

	it('offers the All option for a filter', async () => {
		const user = userEvent.setup();
		const onchange = vi.fn();
		render(RecordPicker, {
			props: { label: 'Profile', options, value: 'p2', allLabel: 'All', onchange }
		});

		await user.click(field());
		await waitFor(() => expect(search()).toHaveFocus());
		await user.click(option('All'));

		expect(onchange).toHaveBeenCalledWith('');
		await waitFor(() => expect(field()).toHaveTextContent('All'));
	});

	it('shows the empty text when the search matches nothing', async () => {
		const user = userEvent.setup();
		render(RecordPicker, {
			props: { label: 'Profile', options, value: '', emptyLabel: 'Nothing found' }
		});

		await user.click(field());
		await user.type(search(), 'zzz');
		expect(await screen.findByText('Nothing found')).toBeInTheDocument();
	});

	it('shows Not set for an empty locked field and does not open', async () => {
		const user = userEvent.setup();
		render(RecordPicker, {
			props: { label: 'Profile', options, value: '', disabled: true, placeholder: 'e.g. x' }
		});

		expect(field()).toBeDisabled();
		expect(field()).toHaveTextContent('Not set');
		expect(field()).not.toHaveTextContent('e.g. x');
		await user.click(field());
		expect(listbox()).not.toBeInTheDocument();
	});

	it('shows a disabled option with its reason and does not select it', async () => {
		const user = userEvent.setup();
		const onchange = vi.fn();
		const hosts: RecordOption[] = [
			{ value: 'h1', label: 'fs-01', dot: 'success', state: 'Online', detail: 'Windows · Online' },
			{
				value: 'h2',
				label: 'db-01',
				dot: 'neutral',
				state: 'Offline',
				disabled: true,
				reason: 'Repository local-fs is on fs-01'
			}
		];
		render(RecordPicker, {
			props: {
				label: 'Profile',
				options: hosts,
				value: 'h1',
				countLabel: (count: number) => `${count} hosts`,
				onchange
			}
		});

		expect(field()).toHaveTextContent(/fs-01\s*· Windows · Online/);
		await user.click(field());
		await waitFor(() => expect(search()).toHaveFocus());
		expect(screen.getByText('2 hosts')).toBeInTheDocument();
		const blocked = option('db-01');
		expect(blocked).toHaveAttribute('data-disabled');
		expect(blocked).toHaveAccessibleDescription(/Offline.*Repository local-fs is on fs-01/);
		expect(option('fs-01')).not.toHaveTextContent('Online');

		await user.click(blocked);
		expect(onchange).not.toHaveBeenCalled();
		await user.type(search(), 'db');
		expect(screen.getByText('1 of 2')).toBeInTheDocument();
	});

	it('marks the field invalid and describes it with the error', () => {
		render(RecordPicker, {
			props: { label: 'Profile', options, value: 'p1', error: 'Choose a profile.' }
		});

		expect(field()).toHaveAttribute('aria-invalid', 'true');
		expect(field()).toHaveAccessibleDescription(/Choose a profile\./);
	});
});
