/*
 * Display rules of the secrets screen. The control plane enforces the same rules (FR-RP-05).
 */
import type { Secret, SecretUsage, SecretUsageKind } from '#lib/api/index.ts';
import type { MessageKey } from '#lib/i18n/index.ts';

/** The purposes that the secrets API accepts on create. The control plane makes the others. */
export const CREATABLE_PURPOSES = [
	'hook_secret',
	'repository_password',
	's3_access_key',
	's3_secret_key',
	'rest_server_password'
] as const;

export type CreatablePurpose = (typeof CREATABLE_PURPOSES)[number];

const PURPOSE_KEYS: Record<string, MessageKey> = {
	repository_password: 'secrets.purpose.repository_password',
	s3_access_key: 'secrets.purpose.s3_access_key',
	s3_secret_key: 'secrets.purpose.s3_secret_key',
	rest_server_password: 'secrets.purpose.rest_server_password',
	hook_secret: 'secrets.purpose.hook_secret',
	webhook_secret: 'secrets.purpose.webhook_secret',
	webhook_url: 'secrets.purpose.webhook_url'
};

/** The message key of a purpose, or null for a purpose this version does not know. */
export function purposeKey(purpose: string): MessageKey | null {
	return PURPOSE_KEYS[purpose] ?? null;
}

export const USAGE_KEYS: Record<SecretUsageKind, MessageKey> = {
	repository: 'secrets.usage.repository',
	profile: 'secrets.usage.profile',
	channel: 'secrets.usage.channel',
	job_template: 'secrets.usage.jobTemplate'
};

export function usageHref(
	usage: SecretUsage
):
	| '/(app)/repositories'
	| '/(app)/profiles'
	| '/(app)/settings/notifications/channels'
	| '/(app)/settings/scripts/jobs' {
	switch (usage.kind) {
		case 'repository':
			return '/(app)/repositories';
		case 'profile':
			return '/(app)/profiles';
		case 'channel':
			return '/(app)/settings/notifications/channels';
		case 'job_template':
			return '/(app)/settings/scripts/jobs';
	}
}

/**
 * The control plane refuses a replace of the password of a repository, because a new value would
 * not open the repository, and of a secret that it made itself for a webhook channel.
 */
export function canReplaceValue(secret: Secret): boolean {
	if (secret.purpose === 'webhook_secret' || secret.purpose === 'webhook_url') return false;
	return !(
		secret.purpose === 'repository_password' &&
		secret.used_by.some((usage) => usage.kind === 'repository')
	);
}

/** True when a replace changed the value after the create. */
export function wasReplaced(secret: Secret): boolean {
	return secret.updated_at !== secret.created_at;
}
