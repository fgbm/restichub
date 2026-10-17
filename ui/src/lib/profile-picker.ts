import type { Agent, Profile, Repository, RepositoryBackendKind } from '#lib/api/index.ts';
import type { RecordOption, RecordPart } from '#lib/components/ui/index.ts';
import { t } from '#lib/i18n/index.ts';
import type { MessageKey } from '#lib/i18n/index.ts';

const BACKEND_TAGS: Record<RepositoryBackendKind, MessageKey> = {
	s3: 'profilePicker.backend.s3',
	rest: 'profilePicker.backend.rest',
	local: 'profilePicker.backend.local'
};

type ProfileRef = Pick<Profile, 'id' | 'name' | 'repository_id'>;
type RepositoryRef = Pick<Repository, 'id' | 'name' | 'backend' | 'maintenance_agent_id'>;
type AgentRef = Pick<Agent, 'id' | 'hostname'>;

/** The repository of a profile, its backend and, for a local path, the host that holds it. */
export function profileDescription(
	profile: ProfileRef,
	repositories: readonly RepositoryRef[],
	agents: readonly AgentRef[]
): RecordPart[] {
	const repository = repositories.find((item) => item.id === profile.repository_id);
	if (!repository) return [];
	const parts: RecordPart[] = [repository.name, { tag: t(BACKEND_TAGS[repository.backend.kind]) }];
	if (repository.backend.kind === 'local') {
		const host = agents.find((agent) => agent.id === repository.maintenance_agent_id);
		parts.push(host ? t('profilePicker.host', { name: host.hostname }) : t('profilePicker.noHost'));
	}
	return parts;
}

export function profileOptions(
	profiles: readonly ProfileRef[],
	repositories: readonly RepositoryRef[],
	agents: readonly AgentRef[]
): RecordOption[] {
	return profiles.map((profile) => ({
		value: profile.id,
		label: profile.name,
		description: profileDescription(profile, repositories, agents)
	}));
}
