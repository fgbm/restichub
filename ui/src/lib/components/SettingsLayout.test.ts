import { render, screen } from '@testing-library/svelte';
import { createRawSnippet, type Snippet } from 'svelte';
import { describe, expect, it, vi } from 'vitest';
import SettingsLayout from './SettingsLayout.svelte';

const state = vi.hoisted(() => ({
	role: 'admin' as string,
	pathname: '/settings/scripts/jobs'
}));

vi.mock('$app/paths', () => ({
	resolve: (path: string) => path.replace('/(app)', '')
}));
vi.mock('$app/state', () => ({
	page: {
		url: {
			get pathname() {
				return state.pathname;
			}
		}
	}
}));
vi.mock('#lib/stores/session.svelte.ts', () => ({ getSession: () => ({ role: state.role }) }));

function html(markup: string): Snippet {
	return createRawSnippet(() => ({ render: () => markup }));
}

describe('SettingsLayout', () => {
	it('shows_the_subnav_and_the_content_column_of_a_role_with_settings', () => {
		state.role = 'admin';
		state.pathname = '/settings/scripts/jobs';
		render(SettingsLayout, { props: { children: html('<p>Body</p>') } });

		const nav = screen.getByRole('navigation', { name: 'Settings sections' });
		expect(nav).toBeInTheDocument();
		expect(nav.parentElement?.classList.contains('settings')).toBe(true);
		expect(nav.parentElement?.classList.contains('settings--no-subnav')).toBe(false);
		expect(screen.getByText('Body')).toBeInTheDocument();
	});

	it('marks_the_list_of_a_record_as_a_parent_page_and_not_as_the_page', () => {
		state.role = 'admin';
		state.pathname = '/settings/scripts/jobs/tpl_01';
		render(SettingsLayout, { props: { children: html('<p>Body</p>') } });

		// The breadcrumb holds the current page, and the subnav holds the list of the record.
		expect(screen.getByRole('link', { name: 'Job templates' })).toHaveAttribute(
			'aria-current',
			'true'
		);
		expect(screen.getByRole('link', { name: 'Users' }).getAttribute('aria-current')).toBeNull();
	});

	it('marks_the_list_itself_as_the_page', () => {
		state.role = 'admin';
		state.pathname = '/settings/scripts/jobs';
		render(SettingsLayout, { props: { children: html('<p>Body</p>') } });

		expect(screen.getByRole('link', { name: 'Job templates' })).toHaveAttribute(
			'aria-current',
			'page'
		);
	});

	it('shows_no_subnav_and_one_column_for_an_operator', () => {
		state.role = 'operator';
		state.pathname = '/settings/scripts/jobs';
		render(SettingsLayout, { props: { children: html('<p>Body</p>') } });

		expect(screen.queryByRole('navigation', { name: 'Settings sections' })).toBeNull();
		const settings = screen.getByText('Body').closest('.settings');
		expect(settings).not.toBeNull();
		expect(settings?.className).toContain('settings--no-subnav');
	});
});
