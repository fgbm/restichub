/*
 * The About ResticHub screen shows the version, the revision and the protocol versions to every
 * role, and the database schema level to the Platform Admin only (AC-AU-02, ADR-0054).
 */
import { render, screen } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { About } from '#lib/api/index.ts';
import { setLocale } from '#lib/i18n/index.ts';
import AboutPage from './+page.svelte';

const state = vi.hoisted(() => ({
	about: null as About | null,
	failed: false,
	platformAdmin: false,
	tenancy: 'multi' as 'single' | 'multi'
}));

vi.mock('#lib/stores/about.svelte.ts', () => ({
	getAbout: () => state.about,
	isAboutFailed: () => state.failed,
	loadAbout: () => Promise.resolve(),
	retryAbout: () => Promise.resolve()
}));

vi.mock('#lib/stores/session.svelte.ts', () => ({
	getSession: () => ({
		user: { id: 'u_01', username: 'root', display_name: 'Root', role: 'platform_admin' },
		tenant: null,
		role: 'platform_admin',
		platform_admin: state.platformAdmin
	})
}));

vi.mock('#lib/stores/instance.svelte.ts', () => ({
	getTenancyMode: () => state.tenancy
}));

vi.mock('$app/paths', () => ({ resolve: (path: string) => path }));

/** A body that the control plane can send for the version `0.2.0-rc.2`. */
function about(overrides: Partial<About> = {}): About {
	return {
		version: '0.2.0-rc.2',
		prerelease: true,
		revision: '3b21922',
		protocol: { major: 1, minor: 1 },
		agent_protocol: { major: 1, minimum_minor: 0 },
		...overrides
	};
}

beforeEach(() => {
	state.about = about();
	state.failed = false;
	state.platformAdmin = false;
	state.tenancy = 'multi';
	setLocale('en');
});

describe('the About ResticHub screen', () => {
	it('adr_0054_shows_the_version_and_the_protocol_of_the_control_plane', () => {
		render(AboutPage);
		expect(screen.getByRole('heading', { level: 1, name: 'About ResticHub' })).toBeInTheDocument();
		// The version keeps its pre-release part, so it matches the release notes exactly.
		expect(screen.getByText('0.2.0-rc.2')).toBeInTheDocument();
		expect(screen.getByText('Pre-release')).toBeInTheDocument();
		expect(screen.getByText('3b21922')).toBeInTheDocument();
		expect(screen.getByText('1.1')).toBeInTheDocument();
		expect(screen.getByText('1.0 and later within major 1')).toBeInTheDocument();
		expect(screen.getByLabelText('Support text')).toHaveValue(
			'ResticHub 0.2.0-rc.2+3b21922 · protocol 1.1 · agents 1.0 and later within major 1'
		);
	});

	it('adr_0054_marks_a_release_build_without_the_badge', () => {
		state.about = about({ version: '0.2.0', prerelease: false });
		render(AboutPage);
		expect(screen.getByText('0.2.0')).toBeInTheDocument();
		expect(screen.queryByText('Pre-release')).not.toBeInTheDocument();
	});

	it('adr_0054_shows_the_installation_facts_of_the_platform_admin', () => {
		state.platformAdmin = true;
		state.about = about({ schema_level: 60 });
		render(AboutPage);
		expect(screen.getByText('Several organizations')).toBeInTheDocument();
		expect(screen.getByText('60')).toBeInTheDocument();
	});

	it('adr_0054_shows_a_missing_schema_level_as_not_set', () => {
		state.platformAdmin = true;
		state.about = about({ schema_level: null });
		render(AboutPage);
		expect(screen.getByText('Database schema level')).toBeInTheDocument();
		expect(screen.getByText('Not set')).toBeInTheDocument();
	});

	it('adr_0054_shows_no_installation_facts_to_another_role', () => {
		render(AboutPage);
		expect(screen.queryByText('Database schema level')).not.toBeInTheDocument();
		expect(screen.queryByText('Several organizations')).not.toBeInTheDocument();
	});

	it('adr_0054_links_the_documentation_of_the_current_language', () => {
		setLocale('ru');
		render(AboutPage);
		expect(screen.getByRole('link', { name: 'Документация' })).toHaveAttribute(
			'href',
			'https://fgbm.github.io/restichub/ru/'
		);
		expect(
			screen.getByRole('link', {
				name: 'Версии агентов, которые принимает этот сервер управления'
			})
		).toHaveAttribute(
			'href',
			'https://fgbm.github.io/restichub/ru/reference/agent-version-compatibility/'
		);
	});
});
