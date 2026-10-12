import { render, screen } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import JsonDiff from './JsonDiff.svelte';
import type { AuditDiff } from '#lib/api/index.ts';

describe('JsonDiff', () => {
	it('renders the old and new values of each field', () => {
		const diff: AuditDiff = {
			disabled: { old: false, new: true },
			name: { old: 'old', new: 'new' }
		};
		render(JsonDiff, { props: { diff } });

		expect(screen.getByText('disabled')).toBeInTheDocument();
		expect(screen.getByText('name')).toBeInTheDocument();
		expect(screen.getByText('old')).toBeInTheDocument();
		expect(screen.getByText('new')).toBeInTheDocument();
		expect(screen.getAllByText('false').length).toBeGreaterThan(0);
		expect(screen.getAllByText('true').length).toBeGreaterThan(0);
	});

	it('renders a redacted value as a muted badge', () => {
		const diff: AuditDiff = {
			value: { old: null, new: '<redacted>' }
		};
		render(JsonDiff, { props: { diff } });

		expect(screen.getByText('redacted')).toBeInTheDocument();
		expect(screen.queryByText('<redacted>')).not.toBeInTheDocument();
	});

	it('shows a message when the record has no change', () => {
		render(JsonDiff, { props: { diff: null } });
		expect(screen.getByText('This record has no change.')).toBeInTheDocument();
	});
});
