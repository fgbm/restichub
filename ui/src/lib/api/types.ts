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
 * No membership holds `platform_admin`; it is an account right (ADR-0029).
 */
export function assignableRoles(): Role[] {
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

/** One membership of the caller (ADR-0025). */
export type TenantAccess = components['schemas']['TenantAccessView'];

export type TenantAccessListResponse = components['schemas']['TenantAccessListResponse'];

export type SwitchTenantRequest = components['schemas']['SwitchTenantRequest'];

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

/** One enrolled agent (ADR-0026). */
export type Agent = components['schemas']['AgentView'];

export type AgentStatus = components['schemas']['AgentStatus'];

export type AgentListResponse = components['schemas']['AgentListResponse'];

export type RenameAgentRequest = components['schemas']['RenameAgentRequest'];

export type EnrollmentToken = components['schemas']['EnrollmentTokenView'];

/** The value appears one time only. */
export type EnrollmentTokenCreated = components['schemas']['CreateEnrollmentTokenResponse'];

export type CreateEnrollmentTokenRequest = components['schemas']['CreateEnrollmentTokenRequest'];

export type EnrollmentTokenListResponse = components['schemas']['EnrollmentTokenListResponse'];

/** The backend settings of a repository (FR-RP-01). */
export type BackendSettings = components['schemas']['BackendSettings'];

export type RepositoryBackendKind = BackendSettings['kind'];

/** The presence of the bound secrets. It never holds a value (AC-RP-02). */
export type RepositoryCredentials = components['schemas']['CredentialsView'];

/** One repository (FR-RP-01). */
export type Repository = components['schemas']['RepositoryView'];

export type RepositoryListResponse = components['schemas']['RepositoryListResponse'];

export type CreateRepositoryRequest = components['schemas']['CreateRepositoryRequest'];

export type UpdateRepositoryRequest = components['schemas']['UpdateRepositoryRequest'];

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

/** The derived administrator state of an organization (ADR-0028). */
export type AdminState = components['schemas']['AdminState'];

/** An installation account with its memberships (ADR-0028). */
export type PlatformUser = components['schemas']['PlatformUserView'];

export type PlatformMembership = components['schemas']['PlatformMembershipView'];

export type PlatformUserListResponse = components['schemas']['PlatformUserListResponse'];

export type CreatePlatformUserRequest = components['schemas']['CreateAccountRequest'];

/** One membership of the account create request. */
export type MembershipInput = components['schemas']['MembershipInput'];

export type SetRoleRequest = components['schemas']['SetRoleRequest'];

/** An absent field leaves the value unchanged. */
export type UpdateAccountRequest = components['schemas']['UpdateAccountRequest'];
