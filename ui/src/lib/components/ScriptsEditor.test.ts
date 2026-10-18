/*
 * The script editor keeps two ordered lists, adds a script blank or from a template, reorders it
 * and warns about a host that did not turn on the trust mode (FR-HK-09, FR-HK-12, ADR-0037).
 */
import { render, screen, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import type { Agent, ScriptTemplate, Secret } from '#lib/api/index.ts';
import { blankScript, draftOf } from '#lib/scripts.ts';
import type { ScriptDraft } from '#lib/scripts.ts';
import ScriptsEditor from './ScriptsEditor.svelte';

const TEMPLATE: ScriptTemplate = {
	id: 'builtin:service-stop-bash',
	builtin: true,
	name: 'Stop a service (Linux)',
	description: 'Stops a systemd service.',
	interpreter: 'bash',
	phase: 'before',
	body: 'systemctl stop "$SERVICE_NAME"\n',
	timeout_seconds: 300,
	failure_policy: 'abort_backup',
	parameters: [{ name: 'SERVICE_NAME', label: 'Service', type: 'string' }]
};

const SECRET: Secret = {
	id: 's_1',
	name: 'db.password',
	purpose: 'hook_secret',
	created_at: '2026-01-01T00:00:00Z',
	updated_at: '2026-01-01T00:00:00Z',
	used_by: []
};

function agent(overrides: Partial<Agent> = {}): Agent {
	return {
		id: 'a_1',
		hostname: 'srv-db-01',
		os: 'linux',
		arch: 'amd64',
		agent_version: '0.4.2',
		restic_version: '0.19.0',
		labels: [],
		restore_roots: [],
		hook_allow_list: 'absent',
		hooks: [],
		server_scripts: 'enabled',
		script_interpreters: ['bash', 'sh'],
		certificate_serial: '01',
		certificate_expires_at: '2999-01-01T00:00:00Z',
		status: 'active',
		last_seen_at: null,
		created_at: '2026-01-01T00:00:00Z',
		revoked_at: null,
		...overrides
	} as Agent;
}

function script(name: string, phase: ScriptDraft['phase']): ScriptDraft {
	const draft = blankScript(phase === 'before' ? 'before' : 'after');
	draft.name = name;
	draft.phase = phase;
	draft.body = `echo ${name}\n`;
	draft.open = false;
	return draft;
}

function names(list: HTMLElement): string[] {
	return within(list)
		.queryAllByTestId('script-card')
		.map((card) => card.querySelector('.card__title')?.textContent ?? '');
}

describe('ScriptsEditor', () => {
	it('shows the scripts in two lists, each in its order', () => {
		render(ScriptsEditor, {
			props: {
				scripts: [
					script('dump', 'before'),
					script('cleanup', 'finally'),
					script('check', 'before'),
					script('notify', 'on_error')
				]
			}
		});
		expect(names(screen.getByTestId('scripts-before'))).toEqual(['dump', 'check']);
		expect(names(screen.getByTestId('scripts-after'))).toEqual(['cleanup', 'notify']);
	});

	it('says that a list is empty', () => {
		render(ScriptsEditor, { props: { scripts: [] } });
		expect(screen.getByText('No script runs before the backup.')).toBeInTheDocument();
		expect(screen.getByText('No script runs after the backup.')).toBeInTheDocument();
	});

	it('adds a blank script to the list where the user asks for it', async () => {
		const user = userEvent.setup();
		render(ScriptsEditor, { props: { scripts: [] } });
		const after = screen.getByTestId('scripts-after');
		await user.click(within(after).getByRole('button', { name: 'Blank script' }));
		expect(within(after).getAllByTestId('script-card')).toHaveLength(1);
		expect(
			within(screen.getByTestId('scripts-before')).queryAllByTestId('script-card')
		).toHaveLength(0);
	});

	it('adds a script from a template with the parameters as variables', async () => {
		const user = userEvent.setup();
		render(ScriptsEditor, { props: { scripts: [], templates: [TEMPLATE] } });
		const before = screen.getByTestId('scripts-before');
		await user.click(within(before).getByRole('button', { name: 'From a script template' }));
		const choices = await screen.findByTestId('template-choices');
		await user.click(within(choices).getByRole('button', { name: 'Use' }));
		const card = within(before).getByTestId('script-card');
		expect(card).toHaveTextContent('Stop a service (Linux)');
		expect(within(card).getByDisplayValue('SERVICE_NAME')).toBeInTheDocument();
		expect(within(card).getByDisplayValue('300')).toBeInTheDocument();
	});

	it('moves a script down inside its list', async () => {
		const user = userEvent.setup();
		render(ScriptsEditor, {
			props: { scripts: [script('first', 'before'), script('second', 'before')] }
		});
		const before = screen.getByTestId('scripts-before');
		const cards = within(before).getAllByTestId('script-card');
		await user.click(within(cards[0]).getByRole('button', { name: 'Move down' }));
		expect(names(before)).toEqual(['second', 'first']);
		expect(
			within(within(before).getAllByTestId('script-card')[0]).getByRole('button', {
				name: 'Move up'
			})
		).toBeDisabled();
	});

	it('removes a script', async () => {
		const user = userEvent.setup();
		render(ScriptsEditor, {
			props: { scripts: [script('first', 'before'), script('second', 'before')] }
		});
		const before = screen.getByTestId('scripts-before');
		const cards = within(before).getAllByTestId('script-card');
		await user.click(within(cards[0]).getByRole('button', { name: 'Remove the script' }));
		expect(names(before)).toEqual(['second']);
	});

	it('shows a secret parameter as a choice of secrets', async () => {
		const user = userEvent.setup();
		const stored = draftOf(
			{
				name: 'dump',
				interpreter: 'bash',
				phase: 'before',
				body: 'pg_dump',
				timeout_seconds: 0,
				failure_policy: 'abort_backup',
				env: [{ name: 'PGPASSWORD', kind: 'secret_ref', value: 's_1' }]
			},
			true
		);
		render(ScriptsEditor, { props: { scripts: [stored], secrets: [SECRET] } });
		const select = screen.getByLabelText('Secret') as HTMLSelectElement;
		expect(select.value).toBe('s_1');
		expect(within(select).getByRole('option', { name: 'db.password' })).toBeInTheDocument();
		await user.click(screen.getByRole('button', { name: 'New secret' }));
		expect(
			await screen.findByText(
				'The control plane keeps the value encrypted and never shows it again.'
			)
		).toBeInTheDocument();
	});

	it('warns about a host without the trust mode and shows the command', () => {
		render(ScriptsEditor, {
			props: {
				scripts: [script('dump', 'before')],
				targets: [agent({ hostname: 'win-fs01', os: 'windows', server_scripts: 'disabled' })]
			}
		});
		expect(screen.getByText('A host does not accept scripts')).toBeInTheDocument();
		expect(
			screen.getByText('These hosts did not turn on the trust mode: win-fs01.')
		).toBeInTheDocument();
		expect(
			screen.getByDisplayValue('restichub-agent.exe trust-server-scripts --enable')
		).toBeInTheDocument();
	});

	it('warns about a host without the interpreter of a script', () => {
		const draft = script('dump', 'before');
		draft.interpreter = 'powershell';
		render(ScriptsEditor, {
			props: { scripts: [draft], targets: [agent({ hostname: 'srv-db-01' })] }
		});
		expect(
			screen.getByText('These hosts lack the interpreter of a script: srv-db-01.')
		).toBeInTheDocument();
	});

	it('shows no warning when every target host accepts the scripts', () => {
		render(ScriptsEditor, { props: { scripts: [script('dump', 'before')], targets: [agent()] } });
		expect(screen.queryByText('A host does not accept scripts')).not.toBeInTheDocument();
	});

	it('shows the scripts and no tool to an Operator', () => {
		render(ScriptsEditor, {
			props: { scripts: [script('dump', 'before')], readonly: true }
		});
		expect(screen.getByText('Only an Admin changes the scripts of a profile.')).toBeInTheDocument();
		expect(screen.queryByRole('button', { name: 'Use a job template' })).not.toBeInTheDocument();
		expect(screen.queryByRole('button', { name: 'Remove the script' })).not.toBeInTheDocument();
	});
});

describe('a script variable of the kind "profile parameter" (FR-PF-10, ADR-0047)', () => {
	const DECLARED = [
		{
			name: 'pg_dir',
			kind: 'text' as const,
			required: false,
			default_value: '/var/backups/pg',
			description: 'Directory of the dumps',
			hidden: false
		},
		{
			name: 'pg_password',
			kind: 'secret' as const,
			required: true,
			default_value: null,
			description: 'Backup password',
			hidden: false
		}
	];

	function withVariable(value: string, kind: 'parameter' | 'string' = 'parameter'): ScriptDraft {
		const draft = script('dump', 'before');
		draft.open = true;
		draft.env = [{ name: 'PGPASSWORD', kind, value }];
		return draft;
	}

	it('ac_pf_09_offers_the_declared_parameters_and_names_a_lost_one', async () => {
		const user = userEvent.setup();
		render(ScriptsEditor, {
			props: { scripts: [withVariable('pg_old')], parameters: DECLARED }
		});

		expect(
			await screen.findByText('The parameter pg_old is not declared in the profile.')
		).toBeInTheDocument();

		await user.click(document.querySelector<HTMLElement>('[role="combobox"]')!);
		const option = await screen.findByText('pg_password', {
			selector: '.rh-record__option-label'
		});
		expect(option.closest('[role="option"]')).toHaveTextContent('Secret');
		expect(option.closest('[role="option"]')).toHaveTextContent('Backup password');
		await user.keyboard('{Escape}');
	});

	it('shows the kind of the chosen parameter in the closed field', async () => {
		render(ScriptsEditor, {
			props: { scripts: [withVariable('pg_dir')], parameters: DECLARED }
		});
		expect(await screen.findByRole('combobox', { name: /Parameter/ })).toHaveTextContent('pg_dir');
		expect(screen.queryByText(/is not declared in the profile/)).not.toBeInTheDocument();
	});

	it('shows two kinds of a variable without a profile', async () => {
		render(ScriptsEditor, { props: { scripts: [withVariable('', 'string')] } });

		const kind = (await screen.findByLabelText('Type')) as HTMLSelectElement;
		const values = [...kind.options].map((option) => option.textContent);
		expect(values).toEqual(['Text', 'Secret']);
	});
});
