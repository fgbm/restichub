import { render, screen } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import Select from './Select.svelte';

const options = [
	{ value: 'admin', label: 'Admin' },
	{ value: 'viewer', label: 'Viewer' }
];

describe('Select', () => {
	it('renders the options and the current value', () => {
		render(Select, { props: { label: 'Role', options, value: 'admin' } });
		expect(screen.getByLabelText('Role')).toHaveValue('admin');
		expect(screen.getByRole('option', { name: 'Viewer' })).toBeInTheDocument();
	});

	it('marks the select invalid on an error', () => {
		render(Select, { props: { label: 'Role', options, error: 'Required' } });
		expect(screen.getByLabelText('Role')).toHaveAttribute('aria-invalid', 'true');
	});

	it('keeps the base class next to a class of the caller', () => {
		render(Select, { props: { label: 'Role', options, class: 'extra' } });
		expect(screen.getByLabelText('Role')).toHaveClass('rh-select', 'extra');
	});

	it('says "Not set" instead of the placeholder in an empty disabled select', () => {
		render(Select, {
			props: { label: 'Role', options, placeholder: 'Pick a role', disabled: true }
		});
		expect(screen.getByRole('option', { name: 'Not set' })).toBeInTheDocument();
		expect(screen.queryByRole('option', { name: 'Pick a role' })).not.toBeInTheDocument();
	});

	it('shows the value of a disabled select without "Not set"', () => {
		render(Select, { props: { label: 'Role', options, value: 'viewer', disabled: true } });
		expect(screen.getByLabelText('Role')).toHaveValue('viewer');
		expect(screen.queryByRole('option', { name: 'Not set' })).not.toBeInTheDocument();
	});
});
