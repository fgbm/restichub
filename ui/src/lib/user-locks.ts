/*
 * The reasons that the API refuses a change of a user. The screens disable the action and show the
 * reason before the click; the control plane keeps the same refusals (ADR-0025, ADR-0029).
 */
import type { PlatformUser, User } from '#lib/api/index.ts';
import { t } from '#lib/i18n/index.ts';

export function activePlatformAdmins(accounts: PlatformUser[]): number {
	return accounts.filter((account) => account.platform_admin && !account.disabled).length;
}

/** A Platform Admin resets its own password with the current password only. */
export function platformResetLock(user: PlatformUser, selfId: string | null): string | null {
	return user.id === selfId ? t('users.selfHint') : null;
}

/** The lock of the right and of the disable switch of an account. */
export function platformChangeLock(
	user: PlatformUser,
	selfId: string | null,
	activeAdmins: number
): string | null {
	if (user.id === selfId) return t('users.selfHint');
	if (user.platform_admin && !user.disabled && activeAdmins <= 1) {
		return t('platformUsers.lastAdminHint');
	}
	return null;
}

/** An organization scope cannot change an account with the Platform Admin right. */
function managedLock(user: User): string | null {
	return user.platform_admin ? t('users.platformManagedHint') : null;
}

/** An Admin changes its own password with the current password only. */
export function orgResetLock(user: User, selfId: string | null): string | null {
	if (user.id === selfId) return t('users.selfHint');
	return managedLock(user) ?? (user.shared ? t('users.sharedResetHint') : null);
}

/** The lock of the role and of the disable switch of a member. */
export function orgChangeLock(user: User, selfId: string | null): string | null {
	return managedLock(user) ?? (user.id === selfId ? t('users.selfHint') : null);
}
