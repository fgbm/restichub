import { render, screen } from '@testing-library/svelte';
import { createRawSnippet, type Snippet } from 'svelte';
import { describe, expect, it, vi } from 'vitest';
import SettingsLayout from './SettingsLayout.svelte';

const state = vi.hoisted(() => ({ role: 'admin' as string }));

vi.mock('$app/paths', () => ({
	resolve: (path: string) => path.replace('/(app)', '')
}));
vi.mock('$app/state', () => ({ page: { url: { pathname: '/settings/scripts/jobs' } } }));
vi.mock('#lib/stores/session.svelte.ts', () => ({ getSession: () => ({ role: state.role }) }));

function html(markup: string): Snippet {
	return createRawSnippet(() => ({ render: () => markup }));
}

describe('SettingsLayout', () => {
	it('shows_the_subnav_and_the_content_column_of_a_role_with_settings', () => {
		state.role = 'admin';
		render(SettingsLayout, { props: { children: html('<p>Body</p>') } });

		const nav = screen.getByRole('navigation', { name: 'Settings sections' });
		expect(nav).toBeInTheDocument();
		expect(nav.parentElement?.classList.contains('settings')).toBe(true);
		expect(nav.parentElement?.classList.contains('settings--no-subnav')).toBe(false);
		expect(screen.getByText('Body')).toBeInTheDocument();
	});

	it('shows_no_subnav_and_one_column_for_an_operator', () => {
		state.role = 'operator';
		render(SettingsLayout, { props: { children: html('<p>Body</p>') } });

		expect(screen.queryByRole('navigation', { name: 'Settings sections' })).toBeNull();
		const settings = screen.getByText('Body').closest('.settings');
		expect(settings).not.toBeNull();
		expect(settings?.className).toContain('settings--no-subnav');
	});
});
