/*
 * Domain types of the UI.
 *
 * The control plane API is not final (issues #18, #19 and #20). The audit types
 * come from the placeholder OpenAPI schema. The other types are the UI contract
 * until the generated schema holds them.
 */
import type { components } from './schema';

export type AuditRecord = components['schemas']['AuditRecord'];
export type AuditPage = components['schemas']['AuditList'];

/** Field changes of one audit record. */
export type AuditDiff = Record<string, { old: unknown; new: unknown }>;

export type AuditActorType = 'user' | 'token' | 'system';

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

/** The roles of the control plane (requirements section 2). */
export type Role = 'platform_admin' | 'admin' | 'operator' | 'viewer';

export interface User {
	id: string;
	username: string;
	display_name: string;
	role: Role;
	disabled: boolean;
	created_at: string;
}

export interface Tenant {
	id: string;
	name: string;
	status: 'active' | 'suspended';
	created_at: string;
}

export interface ApiToken {
	id: string;
	name: string;
	/** The display prefix of the token, for example `rh_ab12`. */
	prefix: string;
	created_at: string;
	expires_at: string | null;
	last_used_at: string | null;
}

/** The token value appears in the create answer one time only. */
export interface ApiTokenCreated extends ApiToken {
	value: string;
}

export interface Me {
	user: User;
	tenant: Tenant | null;
	role: Role;
}

export interface SetupStatus {
	setup_required: boolean;
}

export interface SignInRequest {
	tenant: string;
	username: string;
	password: string;
}

export interface SetupRequest {
	setup_token: string;
	username: string;
	password: string;
	display_name: string;
}

export interface CreateUserRequest {
	username: string;
	display_name: string;
	role: Role;
	password: string;
}

export interface CreateTenantRequest {
	name: string;
}

export interface CreateApiTokenRequest {
	name: string;
	expires_at?: string | null;
}
