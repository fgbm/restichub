/*
 * Domain types of the UI.
 *
 * The types come from the generated OpenAPI schema (`schema.d.ts`, built by
 * `npm run gen:api`). The generated `role`, `status`, `purpose` and audit
 * `action` fields are already closed unions, so the UI re-exports them. The
 * generated audit `diff` is an opaque object, so the UI replaces it with its own
 * diff shape.
 */
import type { components } from './schema';

export type Role = components['schemas']['Role'];

export type TenancyMode = components['schemas']['TenancyMode'];

export type Instance = components['schemas']['InstanceResponse'];

/**
 * The roles that a new user or API token may hold.
 *
 * The platform tenant holds the role `platform_admin` only (FR-TN-02). The
 * other roles apply in one tenant. The actor role tells which tenant it is: the
 * role `platform_admin` exists in the platform tenant only.
 */
export function assignableRoles(role: Role | null | undefined): Role[] {
	if (role === 'platform_admin') return ['platform_admin'];
	return ['admin', 'operator', 'viewer'];
}

export type AuditActorType = 'user' | 'api_token' | 'agent' | 'system';

export interface AuditChange {
	old: unknown;
	new: unknown;
}

/**
 * The field changes of one audit record.
 *
 * An update holds one entry for each changed field (`{ field: { old, new } }`).
 * A create or a delete holds one `{ old, new }` pair at the top level. The
 * component `JsonDiff` normalizes both shapes.
 */
export type AuditDiff = Record<string, unknown>;

export type User = components['schemas']['UserView'];

export type Me = components['schemas']['MeResponse'];

export type SetupStatus = components['schemas']['SetupStatusResponse'];

export type SignInRequest = components['schemas']['SignInRequest'];

export type SetupRequest = components['schemas']['SetupRequest'];

export type ChangePasswordRequest = components['schemas']['ChangePasswordRequest'];

export type CreateUserRequest = components['schemas']['CreateUserRequest'];

/** An absent field does not change the value. */
export type UpdateUserRequest = components['schemas']['UpdateUserRequest'];

export type ResetPasswordRequest = components['schemas']['ResetPasswordRequest'];

export type UserListResponse = components['schemas']['UserListResponse'];

export type ApiToken = components['schemas']['ApiTokenView'];

/** The value appears one time only. */
export type ApiTokenCreated = components['schemas']['CreateTokenResponse'];

export type CreateApiTokenRequest = components['schemas']['CreateTokenRequest'];

export type ApiTokenListResponse = components['schemas']['ApiTokenListResponse'];

/**
 * One audit record. The generated `diff` type is an opaque object, so the UI
 * replaces it with the diff shape that the control plane writes.
 */
export type AuditRecord = Omit<components['schemas']['AuditItem'], 'diff'> & {
	diff?: AuditDiff | null;
};

export type AuditPage = Omit<components['schemas']['AuditPageResponse'], 'items'> & {
	items: AuditRecord[];
};

export interface AuditQuery {
	from?: string;
	to?: string;
	action?: string;
	actor_type?: AuditActorType;
	actor_id?: string;
	object_type?: string;
	object_id?: string;
	cursor?: string;
	limit?: number;
}

export type TenantStatus = components['schemas']['TenantStatus'];

export type Tenant = components['schemas']['PlatformTenantView'];

export type TenantListResponse = components['schemas']['TenantListResponse'];

export type CreateTenantRequest = components['schemas']['CreateTenantRequest'];
