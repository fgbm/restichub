import type { Agent, Repository, RepositoryBackendKind } from '#lib/api/index.ts';
import type { FilterOption, RecordOption, RecordPart } from '#lib/components/ui/index.ts';
import { t } from '#lib/i18n/index.ts';
import type { MessageKey } from '#lib/i18n/index.ts';
import { plural } from '#lib/list-format.ts';

const KIND_TAGS: Record<RepositoryBackendKind, MessageKey> = {
	s3: 'repositoryPicker.backend.s3',
	rest: 'repositoryPicker.backend.rest',
	local: 'repositoryPicker.backend.local'
};

export type RepositoryRef = Pick<
	Repository,
	'id' | 'name' | 'backend' | 'maintenance_agent_id' | 'initialized'
>;

type AgentRef = Pick<Agent, 'id' | 'hostname'>;

/** Where the snapshots of a repository lie, without the prefix that one plan adds to it. */
export function repositoryLocation(repository: RepositoryRef): string {
	const backend = repository.backend;
	switch (backend.kind) {
		case 's3':
			return `${backend.endpoint}/${backend.bucket}`;
		case 'rest':
			return backend.endpoint;
		case 'local':
			return backend.path;
	}
}

export function repositoryKind(repository: RepositoryRef): string {
	return t(KIND_TAGS[repository.backend.kind]);
}

/** The repository as restic names it in `-r`: a URL for storage and a path for a local one. */
export function repositoryAddress(repository: RepositoryRef): string {
	const backend = repository.backend;
	switch (backend.kind) {
		case 's3': {
			const endpoint = backend.endpoint.replace(/^https?:\/\//, '');
			const path = [backend.bucket, backend.prefix].filter(Boolean).join('/');
			return `s3:https://${endpoint}/${path}`;
		}
		case 'rest':
			return `rest:${backend.endpoint}`;
		case 'local':
			return backend.path;
	}
}

export function repositoryHostName(
	repository: RepositoryRef,
	agents: readonly AgentRef[]
): string | null {
	if (repository.backend.kind !== 'local') return null;
	return agents.find((agent) => agent.id === repository.maintenance_agent_id)?.hostname ?? null;
}

/** The kind, the location and, for a local path, the host that holds it. */
export function repositoryDescription(
	repository: RepositoryRef,
	agents: readonly AgentRef[]
): RecordPart[] {
	const parts: RecordPart[] = [{ tag: repositoryKind(repository) }, repositoryLocation(repository)];
	if (repository.backend.kind === 'local') {
		const host = repositoryHostName(repository, agents);
		parts.push(host ? t('repositoryPicker.host', { name: host }) : t('repositoryPicker.noHost'));
	}
	return parts;
}

// A repository that an agent did not initialize holds no snapshot, so a plan cannot back up into
// it. The option stays in the list with the reason (ADR-0046, FR-RP-02).
export function repositoryOptions(
	repositories: readonly RepositoryRef[],
	agents: readonly AgentRef[] = []
): RecordOption[] {
	return repositories.map((repository) => ({
		value: repository.id,
		label: repository.name,
		description: repositoryDescription(repository, agents),
		detail: [repositoryKind(repository), repositoryLocation(repository)]
			.concat(
				repository.backend.kind === 'local'
					? [repositoryHostName(repository, agents) ?? t('repositoryPicker.noHost')]
					: []
			)
			.join(' · '),
		disabled: !repository.initialized,
		reason: repository.initialized ? undefined : t('repositoryPicker.notInitialized')
	}));
}

/** Gives repository facet values the repository name and its secondary line. */
export function withRepositories(
	options: readonly FilterOption[],
	repositories: readonly RepositoryRef[],
	agents: readonly AgentRef[] = [],
	keyOf: (repository: RepositoryRef) => string = (repository) => repository.id
): FilterOption[] {
	const byKey = new Map(repositories.map((repository) => [keyOf(repository), repository]));
	return options.map((option) => {
		const repository = byKey.get(option.value);
		return repository
			? {
					...option,
					label: repository.name,
					description: repositoryDescription(repository, agents)
				}
			: option;
	});
}

export function repositoryCount(count: number): string {
	return plural('repositoryPicker.count', count);
}
