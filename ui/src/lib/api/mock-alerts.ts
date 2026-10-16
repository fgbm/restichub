/*
 * The mock of the alert, channel and dashboard endpoints. Development and tests use it only.
 *
 * The mock starts with no alert and no channel. A review capture or an e2e test sets the storage
 * key to see a filled list.
 */
import type {
	Agent,
	Alert,
	AlertKind,
	AlertListResponse,
	AlertQuery,
	AlertRule,
	ChannelList,
	ChannelSaved,
	ChannelTestResult,
	CreateChannelRequest,
	Dashboard,
	DashboardPlan,
	Job,
	NotificationChannel,
	Plan,
	Repository,
	RpoState,
	UpdateAlertRulesRequest,
	UpdateChannelRequest
} from './types';

export const SAMPLE_ALERTS_KEY = 'restichub.mock.alerts';

const ONLINE_WINDOW_MS = 90_000;

const SAMPLE_ONLINE = new Set(['1', '2']);

function iso(minutesAgo: number): string {
	return new Date(Date.now() - minutesAgo * 60_000).toISOString();
}

function problem(field: string, reason: string): Error {
	return Object.assign(new Error(`${field}: ${reason}`), {
		problem: {
			type: 'about:blank',
			title: 'Bad Request',
			status: 400,
			code: 'invalid_argument',
			request_id: '',
			field,
			reason
		}
	});
}

const KINDS: AlertKind[] = [
	'rpo_violated',
	'job_failed',
	'job_dead_letter',
	'job_warnings',
	'agent_offline',
	'stale_lock'
];

function defaultRules(): AlertRule[] {
	return KINDS.map((kind) => ({
		kind,
		enabled: kind !== 'job_warnings',
		threshold_minutes: kind === 'agent_offline' ? 30 : kind === 'stale_lock' ? 60 : null
	}));
}

function sampleEnabled(): boolean {
	return (
		typeof localStorage !== 'undefined' && localStorage.getItem(SAMPLE_ALERTS_KEY) === 'sample'
	);
}

function sampleAlerts(): Alert[] {
	if (!sampleEnabled()) return [];
	return [
		{
			id: 'al_3',
			kind: 'job_failed',
			status: 'open',
			agent_id: '1',
			agent_hostname: 'win-fs01',
			plan_id: 'pl_1',
			plan_name: 'nightly-web',
			repository_id: 'r_1',
			repository_name: 'primary-s3',
			job_id: 'j_s4',
			params: { job_kind: 'backup', error_code: 'config_error' },
			error_text: 'compile_failed',
			url: 'https://backup.example.com/jobs/j_s4',
			opened_at: iso(50),
			resolved_at: null
		},
		{
			id: 'al_2',
			kind: 'agent_offline',
			status: 'open',
			agent_id: '5',
			agent_hostname: 'mac-build-01',
			plan_id: null,
			plan_name: null,
			repository_id: null,
			repository_name: null,
			job_id: null,
			params: { elapsed_seconds: 48 * 3600, threshold_minutes: 30 },
			error_text: null,
			url: null,
			opened_at: iso(60 * 47),
			resolved_at: null
		},
		{
			id: 'al_1',
			kind: 'rpo_violated',
			status: 'resolved',
			agent_id: '1',
			agent_hostname: 'win-fs01',
			plan_id: 'pl_1',
			plan_name: 'nightly-web',
			repository_id: 'r_1',
			repository_name: 'primary-s3',
			job_id: null,
			params: {
				elapsed_seconds: 31 * 3600,
				rpo_seconds: 26 * 3600,
				last_success_at: iso(60 * 111)
			},
			error_text: null,
			url: null,
			opened_at: iso(60 * 80),
			resolved_at: iso(60 * 60)
		}
	];
}

function sampleChannels(): NotificationChannel[] {
	if (!sampleEnabled()) return [];
	return [
		{
			id: 'ch_1',
			name: 'ops-mail',
			kind: 'email',
			enabled: true,
			recipients: ['ops@example.com', 'backup@example.com'],
			url_origin: null,
			created_at: iso(60 * 24 * 7),
			updated_at: iso(60 * 24 * 7)
		},
		{
			id: 'ch_2',
			name: 'ops-hook',
			kind: 'webhook',
			enabled: true,
			recipients: [],
			url_origin: 'https://hooks.example.com',
			created_at: iso(60 * 24 * 3),
			updated_at: iso(60 * 24 * 2)
		}
	];
}

/// The objective of a plan: the stored value, else the value that the service computed. The mock
/// knows the daily default only (ADR-0036).
function effectiveRpoSeconds(plan: Plan): number {
	return plan.rpo_seconds ?? plan.effective_rpo_seconds ?? 26 * 3600;
}

function rpoState(ageSeconds: number, rpoSeconds: number): RpoState {
	if (ageSeconds > rpoSeconds) return 'violated';
	return ageSeconds * 100 >= rpoSeconds * 75 ? 'at_risk' : 'ok';
}

export interface DashboardInput {
	agents: Agent[];
	plans: Plan[];
	repositories: Repository[];
	jobs: Job[];
}

export class MockAlerts {
	private rules: AlertRule[] = defaultRules();
	private channels: NotificationChannel[] = sampleChannels();
	private alerts: Alert[] = sampleAlerts();
	private nextChannel = 3;

	async listAlerts(query: AlertQuery = {}): Promise<AlertListResponse> {
		const status = query.status ?? 'open';
		const limit = query.limit ?? 50;
		const kinds = (query.kind ?? '').split(',').filter(Boolean);
		const search = (query.q ?? '').toLowerCase();
		const matching = this.alerts
			.filter((alert) => status === 'all' || alert.status === status)
			.filter((alert) => kinds.length === 0 || kinds.includes(alert.kind))
			.filter(
				(alert) =>
					!search ||
					[alert.agent_hostname, alert.plan_name, alert.repository_name].some((name) =>
						name?.toLowerCase().includes(search)
					)
			)
			.sort((a, b) => b.opened_at.localeCompare(a.opened_at));
		const start = query.cursor ? Number(query.cursor) : (query.offset ?? 0);
		const page = matching.slice(start, start + limit);
		const next =
			query.offset === undefined && start + limit < matching.length ? String(start + limit) : null;
		return { items: page, next_cursor: next, total: matching.length, total_capped: false };
	}

	async listAlertRules(): Promise<AlertRule[]> {
		return this.rules.map((rule) => ({ ...rule }));
	}

	async updateAlertRules(request: UpdateAlertRulesRequest): Promise<AlertRule[]> {
		for (const item of request.items) {
			const hasThreshold = item.kind === 'agent_offline' || item.kind === 'stale_lock';
			if (!hasThreshold && item.threshold_minutes != null) {
				throw problem('threshold_minutes', 'unsupported');
			}
			if (hasThreshold && item.threshold_minutes != null && item.threshold_minutes < 1) {
				throw problem('threshold_minutes', 'too_short');
			}
		}
		this.rules = this.rules.map((rule) => {
			const change = request.items.find((item) => item.kind === rule.kind);
			if (!change) return rule;
			return {
				kind: rule.kind,
				enabled: change.enabled,
				threshold_minutes: change.threshold_minutes ?? rule.threshold_minutes
			};
		});
		return this.listAlertRules();
	}

	async listChannels(): Promise<ChannelList> {
		return {
			items: this.channels.map((channel) => ({ ...channel })),
			email_configured: true,
			webhook_allow_http: false
		};
	}

	private checkUrl(url: string): void {
		let parsed: URL;
		try {
			parsed = new URL(url);
		} catch {
			throw problem('url', 'invalid_format');
		}
		if (parsed.protocol !== 'https:') throw problem('url', 'unsupported');
		if (/^(localhost|127\.|10\.|192\.168\.|169\.254\.|\[)/.test(parsed.hostname)) {
			throw problem('url', 'reserved');
		}
	}

	async createChannel(request: CreateChannelRequest): Promise<ChannelSaved> {
		if (!request.name.trim()) throw problem('name', 'empty');
		if (this.channels.some((channel) => channel.name === request.name)) {
			throw Object.assign(new Error('conflict'), {
				problem: {
					type: 'about:blank',
					title: 'Conflict',
					status: 409,
					code: 'conflict',
					request_id: ''
				}
			});
		}
		const recipients = request.recipients ?? [];
		if (request.kind === 'email') {
			if (recipients.length === 0) throw problem('recipients', 'empty');
			if (recipients.some((address) => !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(address))) {
				throw problem('recipients', 'invalid_format');
			}
		} else {
			if (!request.url) throw problem('url', 'empty');
			this.checkUrl(request.url);
		}
		const now = new Date().toISOString();
		const channel: NotificationChannel = {
			id: `ch_${this.nextChannel++}`,
			name: request.name,
			kind: request.kind,
			enabled: request.enabled ?? true,
			recipients: request.kind === 'email' ? recipients : [],
			url_origin: request.kind === 'webhook' ? new URL(request.url ?? '').origin : null,
			created_at: now,
			updated_at: now
		};
		this.channels = [...this.channels, channel];
		return {
			channel,
			signing_secret: request.kind === 'webhook' ? 'whsec_mock_9f3a1c7d5b2e4a60' : null
		};
	}

	async updateChannel(id: string, request: UpdateChannelRequest): Promise<ChannelSaved> {
		const current = this.channels.find((channel) => channel.id === id);
		if (!current) throw new Error('not found');
		if (request.url) this.checkUrl(request.url);
		const updated: NotificationChannel = {
			...current,
			name: request.name ?? current.name,
			enabled: request.enabled ?? current.enabled,
			recipients: request.recipients ?? current.recipients,
			url_origin: request.url ? new URL(request.url).origin : current.url_origin,
			updated_at: new Date().toISOString()
		};
		this.channels = this.channels.map((channel) => (channel.id === id ? updated : channel));
		return {
			channel: updated,
			signing_secret:
				request.rotate_secret && current.kind === 'webhook' ? 'whsec_mock_rotated_77c1' : null
		};
	}

	async deleteChannel(id: string): Promise<void> {
		this.channels = this.channels.filter((channel) => channel.id !== id);
	}

	async testChannel(id: string): Promise<ChannelTestResult> {
		const channel = this.channels.find((item) => item.id === id);
		if (!channel) throw new Error('not found');
		if (channel.url_origin?.includes('down')) return { ok: false, error: 'http_500' };
		return { ok: true, error: null };
	}

	async dashboard(input: DashboardInput): Promise<Dashboard> {
		const now = Date.now();
		// The sample installation has no repository screen data, so it lists two repositories.
		const repositories =
			input.repositories.length === 0 && sampleEnabled()
				? [
						{ id: 'r_1', name: 'primary-s3' },
						{ id: 'r_2', name: 'archive' }
					]
				: input.repositories;
		const agents = input.agents.filter((agent) => agent.status === 'active');
		const hosts = agents.map((agent) => ({
			id: agent.id,
			hostname: agent.hostname,
			// The sample installation shows two hosts that talk to the control plane now.
			online:
				!!agent.first_connected_at &&
				((sampleEnabled() && SAMPLE_ONLINE.has(agent.id)) ||
					(agent.last_seen_at ? now - Date.parse(agent.last_seen_at) <= ONLINE_WINDOW_MS : false)),
			waiting: !agent.first_connected_at,
			last_seen_at: agent.last_seen_at ?? null
		}));
		const online = hosts.filter((host) => host.online).length;
		const waiting = hosts.filter((host) => host.waiting).length;
		hosts.sort(
			(a, b) => Number(a.online) - Number(b.online) || a.hostname.localeCompare(b.hostname)
		);

		const plans: DashboardPlan[] = input.plans.map((plan) => {
			const successes = input.jobs
				.filter(
					(job) =>
						job.plan_id === plan.id &&
						job.kind === 'backup' &&
						job.status === 'succeeded' &&
						job.finished_at
				)
				.map((job) => Date.parse(job.finished_at ?? ''))
				.sort((a, b) => b - a);
			const reference = successes[0] ?? Date.parse(plan.created_at);
			const age = Math.max(0, Math.floor((now - reference) / 1000));
			const rpo = effectiveRpoSeconds(plan);
			return {
				id: plan.id,
				name: plan.name,
				agent_hostname: plan.agent_hostname,
				repository_name:
					repositories.find((repository) => repository.id === plan.repository_id)?.name ??
					plan.repository_id,
				enabled: plan.enabled,
				last_success_at: successes[0] ? new Date(successes[0]).toISOString() : null,
				age_seconds: age,
				rpo_seconds: rpo,
				state: rpoState(age, rpo)
			};
		});
		const rank = (plan: DashboardPlan): number =>
			!plan.enabled ? 3 : plan.state === 'violated' ? 0 : plan.state === 'at_risk' ? 1 : 2;
		plans.sort((a, b) => rank(a) - rank(b) || b.age_seconds - a.age_seconds);
		const counted = (state: RpoState): number =>
			plans.filter((plan) => plan.enabled && plan.state === state).length;

		const failed = input.jobs
			.filter((job) => job.status === 'failed' || job.status === 'dead_letter')
			.sort((a, b) => (b.finished_at ?? '').localeCompare(a.finished_at ?? ''))
			.slice(0, 10)
			.map((job) => ({
				id: job.id,
				kind: job.kind,
				status: job.status,
				result_code: job.result_code ?? null,
				agent_hostname: job.agent_hostname ?? '',
				plan_name: job.plan_name ?? null,
				repository_name: job.repository_name ?? null,
				finished_at: job.finished_at ?? job.updated_at
			}));

		const open = this.alerts
			.filter((alert) => alert.status === 'open')
			.sort((a, b) => b.opened_at.localeCompare(a.opened_at));
		return {
			generated_at: new Date(now).toISOString(),
			hosts: {
				total: hosts.length,
				online,
				offline: hosts.length - online - waiting,
				waiting,
				items: hosts
			},
			plans: {
				total: plans.length,
				at_risk: counted('at_risk'),
				violated: counted('violated'),
				items: plans
			},
			repositories: repositories.map((repository, index) => ({
				id: repository.id,
				name: repository.name,
				lock_state: sampleEnabled() && index === 0 ? ('held' as const) : ('free' as const)
			})),
			failed_jobs: failed,
			alerts: { total: open.length, items: open.slice(0, 10) }
		};
	}
}
