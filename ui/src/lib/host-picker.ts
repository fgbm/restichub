import type { Agent } from '#lib/api/index.ts';
import type {
	FilterOption,
	RecordDot,
	RecordOption,
	RecordPart
} from '#lib/components/ui/index.ts';
import { t } from '#lib/i18n/index.ts';
import { ago, plural } from '#lib/list-format.ts';

export type HostRef = Pick<
	Agent,
	| 'id'
	| 'hostname'
	| 'os'
	| 'arch'
	| 'labels'
	| 'status'
	| 'online'
	| 'first_connected_at'
	| 'last_seen_at'
>;

const OS_NAMES: Record<string, string> = { windows: 'Windows', linux: 'Linux', darwin: 'macOS' };

export function osFamily(os: string): string {
	return OS_NAMES[os.toLowerCase()] ?? os;
}

export function hostPlatform(host: Pick<HostRef, 'os' | 'arch'>): string {
	if (host.os.trim() === '') return t('hostPicker.osUnknown');
	return [osFamily(host.os), host.arch].filter(Boolean).join(' · ');
}

/** The connection state as a dot and the same fact in words, so the color is never the only sign. */
export function hostPresence(host: HostRef, now = Date.now()): { dot: RecordDot; label: string } {
	if (host.status === 'revoked') return { dot: 'neutral', label: t('hosts.status.revoked') };
	if (!host.first_connected_at) return { dot: 'warning', label: t('hosts.status.waiting') };
	if (host.online) return { dot: 'success', label: t('hosts.status.online') };
	return {
		dot: 'neutral',
		label: host.last_seen_at
			? t('hostPicker.offlineFor', { time: ago(host.last_seen_at, now) })
			: t('hosts.status.offline')
	};
}

export function hostDescription(host: HostRef): RecordPart[] {
	return [hostPlatform(host), ...host.labels.map((tag) => ({ tag }))];
}

// A revoked host goes last: it stays only so a stored value or a filter can show it.
function revokedLast<T extends Pick<HostRef, 'status'>>(hosts: readonly T[]): T[] {
	return [...hosts].sort((a, b) => Number(a.status === 'revoked') - Number(b.status === 'revoked'));
}

export function hostOptions(
	hosts: readonly HostRef[],
	reason?: (host: HostRef) => string | null | undefined,
	now = Date.now()
): RecordOption[] {
	return revokedLast(hosts).map((host) => {
		const presence = hostPresence(host, now);
		const why = reason?.(host) || undefined;
		return {
			value: host.id,
			label: host.hostname,
			description: hostDescription(host),
			detail: [host.os.trim() ? osFamily(host.os) : '', presence.label].filter(Boolean).join(' · '),
			dot: presence.dot,
			state: presence.label,
			disabled: why !== undefined,
			reason: why
		};
	});
}

function hostFacetDescription(host: HostRef): RecordPart[] {
	const [platform, ...labels] = hostDescription(host);
	return host.status === 'revoked'
		? [platform, t('hosts.status.revoked'), ...labels]
		: [platform, ...labels];
}

/** Gives host facet values the host name and its secondary line; revoked hosts go last. */
export function withHosts(
	options: readonly FilterOption[],
	hosts: readonly HostRef[],
	keyOf: (host: HostRef) => string = (host) => host.id
): FilterOption[] {
	const byKey = new Map(hosts.map((host) => [keyOf(host), host]));
	const described = options.map((option) => {
		const host = byKey.get(option.value);
		return {
			option: host
				? { ...option, label: host.hostname, description: hostFacetDescription(host) }
				: option,
			revoked: host?.status === 'revoked'
		};
	});
	return [
		...described.filter((item) => !item.revoked),
		...described.filter((item) => item.revoked)
	].map((item) => item.option);
}

export function hostCount(count: number): string {
	return plural('hostPicker.count', count);
}
