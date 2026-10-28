/*
 * The About ResticHub screen shows the version, the revision, the protocol versions and the
 * environment of the server to every role, and the database schema level to the Platform Admin
 * only (AC-AU-02, ADR-0054, ADR-0055).
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
	tenancy: 'multi' as 'single' | 'multi',
	instanceLoaded: true
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
	getTenancyMode: () => state.tenancy,
	isInstanceLoaded: () => state.instanceLoaded
}));

vi.mock('$app/paths', () => ({ resolve: (path: string) => path }));

/** The environment of a Linux server that holds the control plane. */
function environment(): About['environment'] {
	return {
		os: 'linux',
		os_version: 'Ubuntu 24.04.1 LTS',
		kernel: '6.8.0-45-generic',
		arch: 'x86_64',
		container: false,
		database: '16.4'
	};
}

/** A body that the control plane can send for the version `0.2.0-rc.2`. */
function about(overrides: Partial<About> = {}): About {
	return {
		version: '0.2.0-rc.2',
		prerelease: true,
		revision: '3b21922',
		protocol: { major: 1, minor: 1 },
		agent_protocol: { major: 1, minimum_minor: 0 },
		environment: environment(),
		...overrides
	};
}

beforeEach(() => {
	state.about = about();
	state.failed = false;
	state.platformAdmin = false;
	state.tenancy = 'multi';
	state.instanceLoaded = true;
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

	it('adr_0055_shows_the_environment_of_the_server_to_every_role', () => {
		render(AboutPage);
		expect(screen.getByText('Operating system')).toBeInTheDocument();
		expect(screen.getByText('linux')).toBeInTheDocument();
		expect(screen.getByText('Distribution')).toBeInTheDocument();
		expect(screen.getByText('Ubuntu 24.04.1 LTS')).toBeInTheDocument();
		expect(screen.getByText('Kernel')).toBeInTheDocument();
		expect(screen.getByText('6.8.0-45-generic')).toBeInTheDocument();
		expect(screen.getByText('x86_64')).toBeInTheDocument();
		expect(screen.getByText('No')).toBeInTheDocument();
		expect(screen.getByText('16.4')).toBeInTheDocument();
	});

	it('adr_0055_shows_a_fact_that_the_process_does_not_know_as_not_set', () => {
		state.about = about({
			environment: {
				os: 'linux',
				os_version: null,
				kernel: null,
				arch: 'x86_64',
				container: null,
				database: null
			}
		});
		render(AboutPage);
		// A locked fact shows its value or "Not set"; the row never goes away (ui-principles 3.9).
		expect(screen.getByText('Distribution')).toBeInTheDocument();
		expect(screen.getByText('Kernel')).toBeInTheDocument();
		expect(screen.getByText('Container')).toBeInTheDocument();
		expect(screen.getByText('PostgreSQL')).toBeInTheDocument();
		expect(screen.getAllByText('Not set')).toHaveLength(4);
	});

	it('adr_0055_the_support_text_names_a_container_that_the_process_cannot_name', () => {
		state.about = about({
			environment: { ...environment(), kernel: null, container: null }
		});
		render(AboutPage);
		expect(screen.getByLabelText('Support text')).toHaveValue(
			[
				'ResticHub 0.2.0-rc.2+3b21922',
				'protocol 1.1 · agents 1.0 and later within major 1',
				'OS Ubuntu 24.04.1 LTS (linux) · x86_64 · container: not set',
				'PostgreSQL 16.4',
				'organizations: several'
			].join('\n')
		);
	});

	it('adr_0055_the_support_text_holds_the_environment_of_the_server', () => {
		render(AboutPage);
		expect(screen.getByText('Ubuntu 24.04.1 LTS')).toBeInTheDocument();
		expect(screen.getByText('6.8.0-45-generic')).toBeInTheDocument();
		expect(screen.getByText('x86_64')).toBeInTheDocument();
		expect(screen.getByText('16.4')).toBeInTheDocument();
		expect(screen.getByLabelText('Support text')).toHaveValue(
			[
				'ResticHub 0.2.0-rc.2+3b21922',
				'protocol 1.1 · agents 1.0 and later within major 1',
				'OS Ubuntu 24.04.1 LTS (linux) · kernel 6.8.0-45-generic · x86_64 · container: no',
				'PostgreSQL 16.4',
				'organizations: several'
			].join('\n')
		);
	});

	it('adr_0055_the_support_text_names_a_missing_kernel_and_the_single_mode', () => {
		state.tenancy = 'single';
		state.about = about({ environment: { ...environment(), kernel: null, os_version: null } });
		render(AboutPage);
		expect(screen.getByLabelText('Support text')).toHaveValue(
			[
				'ResticHub 0.2.0-rc.2+3b21922',
				'protocol 1.1 · agents 1.0 and later within major 1',
				'OS linux · x86_64 · container: no',
				'PostgreSQL 16.4',
				'organizations: one'
			].join('\n')
		);
	});

	it('adr_0055_the_support_text_waits_for_the_tenancy_mode', () => {
		state.instanceLoaded = false;
		render(AboutPage);
		// The store holds the default mode `multi` before the route answers, so the line is absent.
		expect(screen.getByLabelText('Support text')).toHaveValue(
			[
				'ResticHub 0.2.0-rc.2+3b21922',
				'protocol 1.1 · agents 1.0 and later within major 1',
				'OS Ubuntu 24.04.1 LTS (linux) · kernel 6.8.0-45-generic · x86_64 · container: no',
				'PostgreSQL 16.4'
			].join('\n')
		);
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
