/*
 * The one status vocabulary of the lists (design/style/ui-principles.md, 3 and 4): the label, the
 * tone and the Lucide icon of each entity state, and the type icon of each entity.
 *
 * Color marks only the states that need an action. A usual state, success included, has no pill
 * and a grey icon.
 */
import type {
	Agent,
	Alert,
	AlertKind,
	ApiToken,
	Job,
	NotificationChannel,
	Plan,
	PlatformUser,
	Repository,
	Secret,
	Tenant,
	User
} from '#lib/api/index.ts';
import type { IconName, ListIcon, ListPill, ListTone } from '#lib/components/ui/index.ts';
import { t } from '#lib/i18n/index.ts';
import type { MessageKey } from '#lib/i18n/index.ts';

/** `normal` is the usual state: no pill and a grey icon. */
export type Tone = ListTone | 'normal';

export interface EntityState {
	/** A stable name of the state, for a test or a filter. */
	key: string;
	label: string;
	tone: Tone;
	icon: IconName;
	spin?: boolean;
	/** The full meaning when the label is short, for the tooltip and the accessible name. */
	full?: string;
}

const DAY_MS = 86_400_000;

function state(key: string, label: MessageKey, tone: Tone, icon: IconName): EntityState {
	return { key, label: t(label), tone, icon };
}

/** The icon of the state, colored only for the danger, warning and running tones. */
export function iconOf(entity: EntityState): ListIcon {
	return {
		name: entity.icon,
		tone: entity.tone === 'normal' ? undefined : entity.tone,
		spin: entity.spin,
		label: entity.full ?? entity.label
	};
}

/** The pill of the state; a usual state has none. */
export function pillOf(entity: EntityState): ListPill | null {
	if (entity.tone === 'normal') return null;
	return { tone: entity.tone, label: entity.label, full: entity.full };
}

export const ENTITY_ICONS = {
	overview: 'overview',
	host: 'host',
	repository: 'repository',
	profile: 'checklist',
	plan: 'calendar',
	job: 'job',
	restore: 'restore',
	snapshot: 'camera',
	alert: 'bell',
	audit: 'audit',
	user: 'user',
	apiToken: 'key',
	secret: 'lock',
	email: 'mail',
	webhook: 'webhook',
	scriptTemplate: 'square-terminal',
	jobTemplate: 'list-ordered',
	tenant: 'building-2',
	enrollmentToken: 'ticket'
} as const satisfies Record<string, IconName>;

/** A cancelled job ends as `failed` with the result `cancelled`. */
export function isCancelled(job: Pick<Job, 'status' | 'result_code'>): boolean {
	return job.status === 'failed' && job.result_code === 'cancelled';
}

export function hasWarnings(job: Pick<Job, 'status' | 'result_code'>): boolean {
	return job.status === 'succeeded' && job.result_code === 'succeeded_with_warnings';
}

export function jobState(job: Pick<Job, 'status' | 'result_code'>): EntityState {
	if (isCancelled(job))
		return state('cancelled', 'jobs.status.cancelled', 'neutral', 'circle-slash');
	if (hasWarnings(job)) {
		return {
			...state('warnings', 'jobs.status.succeeded_with_warnings', 'warning', 'check-circle'),
			full: t('jobs.statusFull.succeeded_with_warnings')
		};
	}
	switch (job.status) {
		case 'queued':
			return state('queued', 'jobs.status.queued', 'neutral', 'clock');
		case 'sent':
			return state('sent', 'jobs.status.sent', 'neutral', 'send-horizontal');
		case 'running':
			return { ...state('running', 'jobs.status.running', 'running', 'loader-circle'), spin: true };
		case 'succeeded':
			return state('succeeded', 'jobs.status.succeeded', 'normal', 'check-circle');
		case 'expired':
			return state('expired', 'jobs.status.expired', 'warning', 'clock-alert');
		case 'dead_letter':
			return state('dead_letter', 'jobs.status.dead_letter', 'danger', 'octagon-x');
		default:
			return state('failed', 'jobs.status.failed', 'danger', 'x-circle');
	}
}

const CERTIFICATE_WARNING_MS = 3 * DAY_MS;

/** One pill by priority; the other facts are on the host page. */
export function hostState(agent: Agent, now = Date.now()): EntityState {
	if (agent.status === 'revoked')
		return state('revoked', 'hosts.status.revoked', 'neutral', 'monitor-x');
	if (!agent.first_connected_at) {
		return state('waiting', 'hosts.status.waiting', 'neutral', 'hourglass');
	}
	if (!agent.online) return state('offline', 'hosts.status.offline', 'warning', 'monitor-off');
	const expires = new Date(agent.certificate_expires_at).getTime();
	if (expires <= now) return state('certExpired', 'hosts.status.certExpired', 'danger', 'host');
	if (expires - now < CERTIFICATE_WARNING_MS) {
		return state('certExpiring', 'hosts.status.certExpiring', 'warning', 'host');
	}
	if (agent.possible_duplicate_of) {
		return state('duplicate', 'hosts.status.duplicate', 'warning', 'host');
	}
	if (agent.version_status === 'unsupported') {
		return state('outdated', 'hosts.status.outdated', 'warning', 'host');
	}
	return state('online', 'hosts.status.online', 'normal', 'host');
}

export function planState(plan: Pick<Plan, 'enabled'>): EntityState {
	if (!plan.enabled) return state('disabled', 'plans.disabled', 'neutral', 'calendar-off');
	return state('ok', 'plans.ok', 'normal', 'calendar-check');
}

export function repositoryState(repository: Pick<Repository, 'initialized'>): EntityState {
	if (!repository.initialized) {
		return state('notInitialized', 'repositories.status.notInitialized', 'warning', 'repository');
	}
	return state('ready', 'repositories.status.ready', 'normal', 'repository');
}

const ALERT_KINDS: Record<AlertKind, { tone: ListTone; icon: IconName }> = {
	rpo_violated: { tone: 'danger', icon: 'calendar-x' },
	job_failed: { tone: 'danger', icon: 'x-circle' },
	job_dead_letter: { tone: 'danger', icon: 'octagon-x' },
	job_warnings: { tone: 'warning', icon: 'alert' },
	agent_offline: { tone: 'warning', icon: 'monitor-off' },
	stale_lock: { tone: 'warning', icon: 'lock-keyhole' }
};

/** An open alert has the colored icon of its kind and no pill: the title names the problem. */
export function alertState(alert: Pick<Alert, 'kind' | 'status'>): EntityState {
	const kind = ALERT_KINDS[alert.kind];
	if (alert.status === 'resolved') {
		return state('resolved', 'alerts.status.resolved', 'neutral', kind.icon);
	}
	return { key: 'open', label: t(`alerts.kind.${alert.kind}`), tone: kind.tone, icon: kind.icon };
}

/** An open alert has no pill: its title already names the problem. */
export function alertPill(alert: Pick<Alert, 'kind' | 'status'>): ListPill | null {
	return alert.status === 'resolved' ? pillOf(alertState(alert)) : null;
}

export function channelState(channel: Pick<NotificationChannel, 'enabled' | 'kind'>): EntityState {
	if (!channel.enabled) return state('disabled', 'channels.disabled', 'neutral', 'bell-off');
	return state(
		'enabled',
		'channels.enabled',
		'normal',
		channel.kind === 'email' ? 'mail' : 'webhook'
	);
}

export function userState(user: Pick<User | PlatformUser, 'disabled'>): EntityState {
	if (user.disabled) return state('disabled', 'users.status.disabled', 'neutral', 'user-x');
	return state('active', 'users.status.active', 'normal', 'user');
}

const TOKEN_WARNING_MS = 14 * DAY_MS;

export function apiTokenState(
	token: Pick<ApiToken, 'expires_at' | 'revoked_at'>,
	now = Date.now()
): EntityState {
	if (token.revoked_at) return state('revoked', 'apiTokens.status.revoked', 'neutral', 'key');
	if (token.expires_at) {
		const left = new Date(token.expires_at).getTime() - now;
		if (left <= 0) return state('expired', 'apiTokens.status.expired', 'neutral', 'key');
		if (left < TOKEN_WARNING_MS)
			return state('expiring', 'apiTokens.status.expiring', 'warning', 'key');
	}
	return state('active', 'apiTokens.status.active', 'normal', 'key');
}

export function secretState(secret: Pick<Secret, 'used_by'>): EntityState {
	if (secret.used_by.length === 0) return state('unused', 'secrets.unused', 'neutral', 'lock');
	return state('used', 'secrets.used', 'normal', 'lock');
}

export function tenantState(tenant: Pick<Tenant, 'status' | 'admin_state'>): EntityState {
	if (tenant.status === 'suspended') {
		return state('suspended', 'tenants.status.suspended', 'neutral', 'building-2');
	}
	if (tenant.admin_state === 'none') {
		return state('noAdmin', 'tenants.status.noAdmin', 'warning', 'building-2');
	}
	return state('active', 'tenants.status.active', 'normal', 'building-2');
}
