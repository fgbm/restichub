/*
 * Domain types of the UI.
 *
 * The types come from the generated OpenAPI schema (`schema.d.ts`, built by
 * `npm run gen:api`). The generated `role` and `status` fields are free strings
 * and the generated audit `diff` is an opaque object, so the UI narrows them to
 * its own types.
 */
import type { components } from './schema';

/** The roles of the control plane (requirements section 2). */
export type Role = 'platform_admin' | 'admin' | 'operator' | 'viewer';

/** The identifier of the platform tenant (requirements section 2, FR-TN-02). */
export const PLATFORM_TENANT_ID = '00000000-0000-7000-8000-000000000000';

/**
 * The roles that a new user or API token may hold in a tenant.
 *
 * The platform tenant holds the role `platform_admin` only (FR-TN-02). The
 * other roles apply in one tenant.
 */
export function assignableRoles(tenantId: string | null | undefined): Role[] {
	if (tenantId === PLATFORM_TENANT_ID) return ['platform_admin'];
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

/** A user. The generated `role` is a free string; the UI knows the four roles. */
export type User = Omit<components['schemas']['UserView'], 'role'> & { role: Role };

/** The current actor and its tenant. */
export type Me = Omit<components['schemas']['MeResponse'], 'role' | 'user'> & {
	role: Role;
	user?: User | null;
};

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
export type UserListResponse = Omit<components['schemas']['UserListResponse'], 'items'> & {
	items: User[];
};

/** An API token in a list response. */
export type ApiToken = Omit<components['schemas']['ApiTokenView'], 'role'> & { role: Role };

/** The API token of a create response. The value appears one time only. */
export type ApiTokenCreated = Omit<components['schemas']['CreateTokenResponse'], 'role'> & {
	role: Role;
};

/** The body of the create-token request. */
export type CreateApiTokenRequest = components['schemas']['CreateTokenRequest'];

/** A list of API tokens. */
export type ApiTokenListResponse = Omit<components['schemas']['ApiTokenListResponse'], 'items'> & {
	items: ApiToken[];
};

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

/** The status of a tenant (issue #20). */
export type TenantStatus = 'active' | 'suspended' | 'deleted';

/**
 * A tenant in the platform administration (issue #20). The generated `status`
 * is a free string; the UI knows the three statuses.
 */
export type Tenant = Omit<components['schemas']['PlatformTenantView'], 'status'> & {
	status: TenantStatus;
};

/** A list of tenants. */
export type TenantListResponse = Omit<components['schemas']['TenantListResponse'], 'items'> & {
	items: Tenant[];
};

/** The body of the create-tenant request. */
export type CreateTenantRequest = components['schemas']['CreateTenantRequest'];
