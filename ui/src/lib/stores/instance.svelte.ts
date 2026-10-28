/*
 * Instance state: the tenancy mode and the setup state.
 *
 * The application reads `/api/v1/instance` at load, so the sign-in and the setup
 * screens hide the tenant field in single mode and the navigation hides the
 * platform section.
 */
import { service } from '#lib/api/index.ts';
import type { TenancyMode } from '#lib/api/index.ts';

// The default keeps the multi-tenant surface when the control plane does not answer.
let mode = $state<TenancyMode>('multi');
let modeKnown = $state(false);
let setupRequired = $state<boolean | null>(null);
let loaded = $state(false);

export function getTenancyMode(): TenancyMode {
	return mode;
}

export function isSingleTenant(): boolean {
	return mode === 'single';
}

/** True after `/api/v1/instance` reported the mode. A failed read leaves the default mode. */
export function isTenancyModeKnown(): boolean {
	return modeKnown;
}

export function getSetupRequired(): boolean | null {
	return setupRequired;
}

export function isInstanceLoaded(): boolean {
	return loaded;
}

export async function loadInstance(): Promise<void> {
	try {
		const instance = await service.instance();
		mode = instance.tenancy_mode;
		setupRequired = instance.setup_required;
		// A default of `multi` is a guess, so the screens show the mode only after the answer
		// (ADR-0055, decision 4).
		modeKnown = true;
	} catch {
		// Keep the default mode when the control plane does not answer.
	} finally {
		loaded = true;
	}
}
