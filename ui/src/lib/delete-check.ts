/*
 * A refused delete names its reason. The dialog asks the usage route before the confirmation, and
 * it reads the same shape from a refused delete when the state changes between the check and the
 * click.
 */
import { resolve } from '$app/paths';
import { ApiError } from '#lib/api/errors.ts';
import type { Blocker, BlockerKind, Blockers, UsageView } from '#lib/api/types.ts';
import { t, tErrorCode, type MessageKey } from '#lib/i18n/index.ts';

export interface BlockedLink {
	label: string;
	kind: string;
	href: string;
}

/** What the confirmation dialog shows for a blocked delete. */
export interface BlockedView {
	code: string;
	reason: string;
	links: BlockedLink[];
	/** The count of the records that the list leaves out. */
	more: number;
}

function href(blocker: Blocker): string {
	const params = { id: blocker.id };
	switch (blocker.kind) {
		case 'repository':
			return resolve('/(app)/repositories/[id]', params);
		case 'profile':
			return resolve('/(app)/profiles/[id]', params);
		case 'plan':
			return resolve('/(app)/plans/[id]', params);
		case 'job':
			return resolve('/(app)/jobs/[id]', params);
		case 'channel':
			return resolve('/(app)/settings/notifications/channels');
		case 'job_template':
			return resolve('/(app)/settings/scripts/jobs/[id]', params);
	}
}

export function kindLabel(kind: BlockerKind): string {
	return t(`deleteBlocked.kind.${kind}` as MessageKey);
}

export function blockedView(code: string, blockers: Blockers | undefined): BlockedView {
	const items = blockers?.items ?? [];
	return {
		code,
		reason: tErrorCode(code) ?? code,
		links: items.map((blocker) => ({
			label: blocker.name,
			kind: kindLabel(blocker.kind),
			href: href(blocker)
		})),
		more: Math.max((blockers?.total ?? 0) - items.length, 0)
	};
}

/** The blocked state of a usage answer, or null when the delete is allowed. */
export function fromUsage(usage: UsageView): BlockedView | null {
	return usage.blocked_by ? blockedView(usage.blocked_by, usage.blockers) : null;
}

/** The blocked state of a delete that the server refused with a list, or null. */
export function fromRefusal(error: unknown): BlockedView | null {
	if (!(error instanceof ApiError) || error.status !== 409) return null;
	if (!error.problem.blockers) return null;
	return blockedView(error.code, error.problem.blockers);
}
