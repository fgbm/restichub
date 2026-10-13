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

/** The roles of the control plane (requirements section 2). */
export type Role = components['schemas']['Role'];

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

/** The actor types of an audit record (the values that the server stores). */
export type AuditActorType = 'user' | 'api_token' | 'agent' | 'system';

/** The change of one field. */
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

/** A user. */
export type User = components['schemas']['UserView'];

/** The current actor and its tenant. */
export type Me = components['schemas']['MeResponse'];

/** The setup status. */
export type SetupStatus = components['schemas']['SetupStatusResponse'];

/** The body of the sign-in request. */
export type SignInRequest = components['schemas']['SignInRequest'];

/** The body of the setup request. */
export type SetupRequest = components['schemas']['SetupRequest'];

/** The body of the self password change request. */
export type ChangePasswordRequest = components['schemas']['ChangePasswordRequest'];

/** The body of the create-user request. */
export type CreateUserRequest = components['schemas']['CreateUserRequest'];

/** The body of the update-user request. An absent field does not change. */
export type UpdateUserRequest = components['schemas']['UpdateUserRequest'];

/** The body of the admin password reset request. */
export type ResetPasswordRequest = components['schemas']['ResetPasswordRequest'];

/** A list of users. */
export type UserListResponse = components['schemas']['UserListResponse'];

/** An API token in a list response. */
export type ApiToken = components['schemas']['ApiTokenView'];

/** The API token of a create response. The value appears one time only. */
export type ApiTokenCreated = components['schemas']['CreateTokenResponse'];

/** The body of the create-token request. */
export type CreateApiTokenRequest = components['schemas']['CreateTokenRequest'];

/** A list of API tokens. */
export type ApiTokenListResponse = components['schemas']['ApiTokenListResponse'];

/**
 * One audit record. The generated `diff` type is an opaque object, so the UI
 * replaces it with the diff shape that the control plane writes.
 */
export type AuditRecord = Omit<components['schemas']['AuditItem'], 'diff'> & {
	diff?: AuditDiff | null;
};

/** One page of audit records with the cursor of the next page. */
export type AuditPage = Omit<components['schemas']['AuditPageResponse'], 'items'> & {
	items: AuditRecord[];
};

/** The query of the audit endpoints. */
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

/** The status of a tenant. */
export type TenantStatus = components['schemas']['TenantStatus'];

/** A tenant in the platform administration. */
export type Tenant = components['schemas']['PlatformTenantView'];

/** A list of tenants. */
export type TenantListResponse = components['schemas']['TenantListResponse'];

/** The body of the create-tenant request. */
export type CreateTenantRequest = components['schemas']['CreateTenantRequest'];
