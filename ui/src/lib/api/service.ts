/*
 * The service layer. One place connects the screens to the API.
 *
 * Set `VITE_USE_MOCK=true` to use the in-memory mock (development and tests).
 * The mock is loaded with a dynamic import, so a production build holds no mock
 * code: Vite replaces the flag with `false` and removes the import.
 */
import { api } from './client';
import { endpoints } from './endpoints';
import type {
	Agent,
	AgentListResponse,
	AgentQuery,
	AlertListResponse,
	AlertQuery,
	AlertRule,
	ApiToken,
	ApiTokenCreated,
	ApiTokenListResponse,
	AuditPage,
	AuditQuery,
	Dashboard,
	ChangePasswordRequest,
	ChannelList,
	ChannelSaved,
	ChannelTestResult,
	CreateChannelRequest,
	CreateApiTokenRequest,
	CreateEnrollmentTokenRequest,
	CreatePlanRequest,
	CreateProfileRequest,
	CreateJobTemplateRequest,
	CreateRepositoryRequest,
	CreateScriptTemplateRequest,
	CreateSecretRequest,
	CreateTenantRequest,
	CreateUserRequest,
	CreatePlatformUserRequest,
	EnrollmentToken,
	ReplaceAgentResponse,
	EnrollmentTokenCreated,
	EnrollmentTokenListResponse,
	Instance,
	Job,
	JobActionRequest,
	JobTemplate,
	JobTemplateListResponse,
	JobDetail,
	JobListResponse,
	JobLimitDefaults,
	JobLogPage,
	JobLogQuery,
	JobQuery,
	LsQueryRequest,
	LsView,
	Maintenance,
	MaintenanceTask,
	Me,
	MembershipInput,
	NextRunsResponse,
	Plan,
	PlanListResponse,
	PlanResponse,
	PlatformUser,
	PlatformUserListResponse,
	PreviewRequest,
	PreviewResponse,
	PreviewScheduleRequest,
	Profile,
	ProfileListResponse,
	ProfileResponse,
	RenameAgentRequest,
	Repository,
	RestoreRequest,
	RepositoryListResponse,
	ResetPasswordRequest,
	SchedulePreviewResponse,
	ScriptTemplate,
	ScriptTemplateListResponse,
	Secret,
	SecretListResponse,
	SetRoleRequest,
	SetupRequest,
	SetupStatus,
	SignInRequest,
	SnapshotsParams,
	SnapshotsQuery,
	SnapshotsView,
	Tenant,
	TenantAccess,
	TenantAccessListResponse,
	TenantListResponse,
	UpdateAccountRequest,
	UpdateAlertRulesRequest,
	UpdateChannelRequest,
	UpdateJobTemplateRequest,
	UpdateScriptTemplateRequest,
	UpdatePlanRequest,
	UpdateProfileRequest,
	UpdateMaintenanceRequest,
	UpdateRepositoryRequest,
	UpdateUserRequest,
	User,
	UserListResponse
} from './types';

export interface ApiService {
	signIn(request: SignInRequest): Promise<Me>;
	signOut(): Promise<void>;
	currentUser(): Promise<Me>;
	changePassword(request: ChangePasswordRequest): Promise<void>;

	/** The memberships of the caller (ADR-0025). */
	listMemberships(): Promise<TenantAccess[]>;
	/** Moves the session to another tenant and returns the new session. */
	switchTenant(tenantId: string): Promise<Me>;

	setupStatus(): Promise<SetupStatus>;
	setup(request: SetupRequest): Promise<Me>;

	instance(): Promise<Instance>;

	listUsers(): Promise<User[]>;
	createUser(request: CreateUserRequest): Promise<User>;
	updateUser(id: string, patch: UpdateUserRequest): Promise<User>;
	deleteUser(id: string): Promise<void>;
	resetUserPassword(id: string, request: ResetPasswordRequest): Promise<void>;

	listTokens(): Promise<ApiToken[]>;
	createToken(request: CreateApiTokenRequest): Promise<ApiTokenCreated>;
	revokeToken(id: string): Promise<void>;

	/** The enrolled agents of the tenant (ADR-0026). */
	listAgents(): Promise<Agent[]>;
	/** One filtered, sorted page of the hosts, with the total (FR-UI-01). */
	searchAgents(query: AgentQuery): Promise<AgentListResponse>;
	getAgent(id: string): Promise<Agent>;
	renameAgent(id: string, hostname: string): Promise<Agent>;
	revokeAgent(id: string): Promise<void>;
	/** A token bound to the host: the enrollment keeps the identifier and the plans (FR-ID-01). */
	reenrollAgent(id: string): Promise<EnrollmentTokenCreated>;
	/** Moves the plans of the old duplicate record to this host and revokes the old record. */
	replaceAgent(id: string, oldAgentId: string): Promise<ReplaceAgentResponse>;

	/** One-time enrollment tokens (FR-ID-01). */
	listEnrollmentTokens(): Promise<EnrollmentToken[]>;
	createEnrollmentToken(request: CreateEnrollmentTokenRequest): Promise<EnrollmentTokenCreated>;
	revokeEnrollmentToken(id: string): Promise<void>;

	/** The repositories of the tenant (FR-RP-01). */
	listRepositories(): Promise<Repository[]>;
	createRepository(request: CreateRepositoryRequest): Promise<Repository>;
	updateRepository(id: string, request: UpdateRepositoryRequest): Promise<Repository>;
	deleteRepository(id: string): Promise<void>;

	/** Starts an init job on an agent (FR-RP-02). */
	initRepository(id: string, request?: JobActionRequest): Promise<Job>;
	/** Starts a test-access job on an agent (FR-RP-02). */
	testAccessRepository(id: string, request?: JobActionRequest): Promise<Job>;
	/** The maintenance schedules and the latest maintenance jobs of a repository (FR-RP-08). */
	getMaintenance(id: string): Promise<Maintenance>;
	/** Replaces the maintenance schedules of a repository (FR-RP-08). */
	updateMaintenance(id: string, request: UpdateMaintenanceRequest): Promise<Maintenance>;
	/** Queues a maintenance task of a repository now (FR-SC-09). */
	runMaintenance(id: string, task: MaintenanceTask): Promise<Job>;
	/** Removes every lock of the repository. `confirm` is the repository name (FR-SC-06). */
	unlockAllRepository(id: string, confirm: string): Promise<Job>;
	/** Starts a job that lists the snapshots of a repository with the filters (FR-RS-01). */
	startSnapshots(id: string, request: SnapshotsQuery): Promise<SnapshotsView>;
	/** Reads the snapshot list of a job, or the newest stored list without a job (FR-RS-01). */
	getSnapshots(id: string, params?: SnapshotsParams): Promise<SnapshotsView>;
	/** Starts a job that reads one page of a snapshot directory. A stored page answers at once. */
	startLs(id: string, request: LsQueryRequest): Promise<LsView>;
	/** Reads the page of a tree job (FR-RS-02). */
	getLs(id: string, job: string): Promise<LsView>;
	/** Starts a restore on an agent (FR-RS-03, FR-RS-04). */
	restoreSnapshot(id: string, request: RestoreRequest): Promise<Job>;
	/** Polls one job (FR-PR-01). */
	getJob(id: string): Promise<JobDetail>;
	/** Reads a page of the log of a job after a line number (FR-LG-01). */
	getJobLogs(id: string, query?: JobLogQuery): Promise<JobLogPage>;
	/** One page of the jobs of the tenant, newest first (FR-SC-09). */
	listJobs(query?: JobQuery): Promise<JobListResponse>;
	/** Cancels a job. A queued job ends at once; a running job gets the cancel message (FR-SC-09). */
	cancelJob(id: string): Promise<Job>;

	/** The profiles of the tenant (FR-PF-01). */
	listProfiles(): Promise<Profile[]>;
	/** The job limits that a new profile gets (ADR-0039). */
	getJobLimitDefaults(): Promise<JobLimitDefaults>;
	createProfile(request: CreateProfileRequest): Promise<ProfileResponse>;
	updateProfile(id: string, request: UpdateProfileRequest): Promise<ProfileResponse>;
	deleteProfile(id: string): Promise<void>;
	/** The compiled steps of a profile for one agent, with masked secrets (FR-PF-04). */
	previewProfile(id: string, agentId: string): Promise<PreviewResponse>;

	/** The plans of the tenant (FR-SC-01). */
	listPlans(): Promise<Plan[]>;
	createPlan(request: CreatePlanRequest): Promise<PlanResponse>;
	updatePlan(id: string, request: UpdatePlanRequest): Promise<PlanResponse>;
	deletePlan(id: string): Promise<void>;
	/** Queues a backup of the plan now (FR-SC-09). */
	runPlan(id: string): Promise<Job>;
	/** The next fire times of a plan, in UTC (FR-SC-01). */
	nextRuns(id: string, count: number): Promise<NextRunsResponse>;
	/** The next fire times of an unsaved schedule, in UTC (FR-SC-01). */
	previewSchedule(request: PreviewScheduleRequest): Promise<SchedulePreviewResponse>;

	/** The metadata of the secrets of the tenant. A value is never returned. */
	listSecrets(): Promise<Secret[]>;
	/** Stores a secret. The value goes to the control plane once and never comes back. */
	createSecret(request: CreateSecretRequest): Promise<Secret>;
	/** Replaces the value of a secret. The old value stays hidden. */
	replaceSecretValue(id: string, value: string): Promise<void>;
	/** Removes a secret. The control plane refuses it while an object uses the secret. */
	deleteSecret(id: string): Promise<void>;

	/** The built-in and the own script templates (FR-HK-12). */
	listScriptTemplates(): Promise<ScriptTemplate[]>;
	createScriptTemplate(request: CreateScriptTemplateRequest): Promise<ScriptTemplate>;
	updateScriptTemplate(id: string, request: UpdateScriptTemplateRequest): Promise<ScriptTemplate>;
	deleteScriptTemplate(id: string): Promise<void>;
	/** The built-in and the own job templates (FR-HK-12). */
	listJobTemplates(): Promise<JobTemplate[]>;
	createJobTemplate(request: CreateJobTemplateRequest): Promise<JobTemplate>;
	updateJobTemplate(id: string, request: UpdateJobTemplateRequest): Promise<JobTemplate>;
	deleteJobTemplate(id: string): Promise<void>;

	listTenants(): Promise<Tenant[]>;
	createTenant(request: CreateTenantRequest): Promise<Tenant>;
	suspendTenant(id: string): Promise<void>;
	resumeTenant(id: string): Promise<void>;
	deleteTenant(id: string): Promise<void>;

	/** Every account of the installation with its memberships (ADR-0028). */
	listPlatformUsers(): Promise<PlatformUser[]>;
	createPlatformUser(request: CreatePlatformUserRequest): Promise<PlatformUser>;
	updatePlatformUser(id: string, patch: UpdateAccountRequest): Promise<void>;
	addPlatformMembership(id: string, request: MembershipInput): Promise<void>;
	setPlatformMembershipRole(id: string, tenantId: string, request: SetRoleRequest): Promise<void>;
	removePlatformMembership(id: string, tenantId: string): Promise<void>;

	/** The overview of the organization: hosts, plans, repositories, failed jobs, alerts (FR-MN-01). */
	getDashboard(): Promise<Dashboard>;
	/** One page of the alerts, newest first (FR-MN-03). */
	listAlerts(query?: AlertQuery): Promise<AlertListResponse>;
	/** The setting of each alert kind (FR-MN-03). */
	listAlertRules(): Promise<AlertRule[]>;
	updateAlertRules(request: UpdateAlertRulesRequest): Promise<AlertRule[]>;
	/** The notification channels of the organization (FR-MN-04). */
	listChannels(): Promise<ChannelList>;
	createChannel(request: CreateChannelRequest): Promise<ChannelSaved>;
	updateChannel(id: string, request: UpdateChannelRequest): Promise<ChannelSaved>;
	deleteChannel(id: string): Promise<void>;
	/** Sends a test message through the channel. */
	testChannel(id: string): Promise<ChannelTestResult>;

	listAudit(query: AuditQuery): Promise<AuditPage>;
	listPlatformAudit(query: AuditQuery): Promise<AuditPage>;
}

const httpService: ApiService = {
	signIn: (request) => api.post<Me>(endpoints.auth.signIn, request),
	signOut: () => api.post<void>(endpoints.auth.signOut),
	currentUser: () => api.get<Me>(endpoints.auth.me),
	changePassword: (request) => api.post<void>(endpoints.auth.password, request),

	listMemberships: () =>
		api.get<TenantAccessListResponse>(endpoints.auth.tenants).then((response) => response.items),
	switchTenant: (tenantId) => api.post<Me>(endpoints.auth.switchTenant, { tenant_id: tenantId }),

	setupStatus: () => api.get<SetupStatus>(endpoints.setup.status),
	setup: (request) => api.post<Me>(endpoints.setup.create, request),

	instance: () => api.get<Instance>(endpoints.instance),

	listUsers: () =>
		api.get<UserListResponse>(endpoints.users.list).then((response) => response.items),
	createUser: (request) => api.post<User>(endpoints.users.create, request),
	updateUser: (id, patch) => api.patch<User>(endpoints.users.byId(id), patch),
	deleteUser: (id) => api.delete<void>(endpoints.users.byId(id)),
	resetUserPassword: (id, request) => api.post<void>(endpoints.users.password(id), request),

	listTokens: () =>
		api.get<ApiTokenListResponse>(endpoints.apiTokens.list).then((response) => response.items),
	createToken: (request) => api.post<ApiTokenCreated>(endpoints.apiTokens.create, request),
	revokeToken: (id) => api.delete<void>(endpoints.apiTokens.byId(id)),

	listAgents: () =>
		api.get<AgentListResponse>(endpoints.agents.list).then((response) => response.items),
	searchAgents: (query) =>
		api.get<AgentListResponse>(endpoints.agents.list, {
			query: { ...query, facets: query.facets ? 'true' : undefined }
		}),
	getAgent: (id) => api.get<Agent>(endpoints.agents.byId(id)),
	renameAgent: (id, hostname) =>
		api.patch<Agent>(endpoints.agents.byId(id), { hostname } satisfies RenameAgentRequest),
	revokeAgent: (id) => api.post<void>(endpoints.agents.revoke(id)),
	reenrollAgent: (id) => api.post<EnrollmentTokenCreated>(endpoints.agents.reenroll(id), {}),
	replaceAgent: (id, oldAgentId) =>
		api.post<ReplaceAgentResponse>(endpoints.agents.replace(id), { old_agent_id: oldAgentId }),

	listEnrollmentTokens: () =>
		api
			.get<EnrollmentTokenListResponse>(endpoints.enrollmentTokens.list)
			.then((response) => response.items),
	createEnrollmentToken: (request) =>
		api.post<EnrollmentTokenCreated>(endpoints.enrollmentTokens.create, request),
	revokeEnrollmentToken: (id) => api.delete<void>(endpoints.enrollmentTokens.byId(id)),

	listRepositories: () =>
		api.get<RepositoryListResponse>(endpoints.repositories.list).then((response) => response.items),
	createRepository: (request) => api.post<Repository>(endpoints.repositories.create, request),
	updateRepository: (id, request) =>
		api.patch<Repository>(endpoints.repositories.byId(id), request),
	deleteRepository: (id) => api.delete<void>(endpoints.repositories.byId(id)),

	initRepository: (id, request) => api.post<Job>(endpoints.repositories.init(id), request ?? {}),
	testAccessRepository: (id, request) =>
		api.post<Job>(endpoints.repositories.testAccess(id), request ?? {}),
	getMaintenance: (id) => api.get<Maintenance>(endpoints.repositories.maintenance(id)),
	updateMaintenance: (id, request) =>
		api.put<Maintenance>(endpoints.repositories.maintenance(id), request),
	runMaintenance: (id, task) => api.post<Job>(endpoints.repositories.runMaintenance(id), { task }),
	unlockAllRepository: (id, confirm) =>
		api.post<Job>(endpoints.repositories.unlockAll(id), { confirm }),
	startSnapshots: (id, request) =>
		api.post<SnapshotsView>(endpoints.repositories.snapshots(id), request),
	getSnapshots: (id, params) =>
		api.get<SnapshotsView>(endpoints.repositories.snapshots(id), {
			query: (params ?? {}) as Record<string, string | undefined>
		}),
	startLs: (id, request) => api.post<LsView>(endpoints.repositories.ls(id), request),
	getLs: (id, job) => api.get<LsView>(endpoints.repositories.ls(id), { query: { job } }),
	restoreSnapshot: (id, request) => api.post<Job>(endpoints.repositories.restore(id), request),
	getJob: (id) => api.get<JobDetail>(endpoints.jobs.byId(id)),
	getJobLogs: (id, query) =>
		api.get<JobLogPage>(endpoints.jobs.logs(id), {
			query: (query ?? {}) as Record<string, string | number | undefined>
		}),
	listJobs: (query) =>
		api.get<JobListResponse>(endpoints.jobs.list, {
			query: (query ?? {}) as Record<string, string | number | undefined>
		}),
	cancelJob: (id) => api.post<Job>(endpoints.jobs.cancel(id)),

	listProfiles: () =>
		api.get<ProfileListResponse>(endpoints.profiles.list).then((response) => response.items),
	getJobLimitDefaults: () => api.get<JobLimitDefaults>(endpoints.profiles.defaults),
	createProfile: (request) => api.post<ProfileResponse>(endpoints.profiles.create, request),
	updateProfile: (id, request) => api.patch<ProfileResponse>(endpoints.profiles.byId(id), request),
	deleteProfile: (id) => api.delete<void>(endpoints.profiles.byId(id)),
	previewProfile: (id, agentId) =>
		api.post<PreviewResponse>(endpoints.profiles.preview(id), {
			agent_id: agentId
		} satisfies PreviewRequest),

	listPlans: () =>
		api.get<PlanListResponse>(endpoints.plans.list).then((response) => response.items),
	createPlan: (request) => api.post<PlanResponse>(endpoints.plans.create, request),
	updatePlan: (id, request) => api.patch<PlanResponse>(endpoints.plans.byId(id), request),
	deletePlan: (id) => api.delete<void>(endpoints.plans.byId(id)),
	runPlan: (id) => api.post<Job>(endpoints.plans.run(id)),
	nextRuns: (id, count) => api.get<NextRunsResponse>(endpoints.plans.nextRuns(id, count)),
	previewSchedule: (request) => api.post<SchedulePreviewResponse>(endpoints.plans.preview, request),

	listSecrets: () =>
		api.get<SecretListResponse>(endpoints.secrets.list).then((response) => response.items),
	createSecret: (request) => api.post<Secret>(endpoints.secrets.create, request),
	replaceSecretValue: (id, value) => api.put<void>(endpoints.secrets.value(id), { value }),
	deleteSecret: (id) => api.delete<void>(endpoints.secrets.byId(id)),

	listScriptTemplates: () =>
		api
			.get<ScriptTemplateListResponse>(endpoints.scriptTemplates.list)
			.then((response) => response.items),
	createScriptTemplate: (request) =>
		api.post<ScriptTemplate>(endpoints.scriptTemplates.create, request),
	updateScriptTemplate: (id, request) =>
		api.patch<ScriptTemplate>(endpoints.scriptTemplates.byId(id), request),
	deleteScriptTemplate: (id) => api.delete<void>(endpoints.scriptTemplates.byId(id)),
	listJobTemplates: () =>
		api
			.get<JobTemplateListResponse>(endpoints.jobTemplates.list)
			.then((response) => response.items),
	createJobTemplate: (request) => api.post<JobTemplate>(endpoints.jobTemplates.create, request),
	updateJobTemplate: (id, request) =>
		api.patch<JobTemplate>(endpoints.jobTemplates.byId(id), request),
	deleteJobTemplate: (id) => api.delete<void>(endpoints.jobTemplates.byId(id)),

	listTenants: () =>
		api.get<TenantListResponse>(endpoints.platform.tenants).then((response) => response.items),
	createTenant: (request) => api.post<Tenant>(endpoints.platform.createTenant, request),
	suspendTenant: (id) => api.post<void>(endpoints.platform.suspendTenant(id)),
	resumeTenant: (id) => api.post<void>(endpoints.platform.resumeTenant(id)),
	deleteTenant: (id) => api.delete<void>(endpoints.platform.deleteTenant(id)),

	listPlatformUsers: () =>
		api.get<PlatformUserListResponse>(endpoints.platform.users).then((response) => response.items),
	createPlatformUser: (request) => api.post<PlatformUser>(endpoints.platform.createUser, request),
	updatePlatformUser: (id, patch) => api.patch<void>(endpoints.platform.userById(id), patch),
	addPlatformMembership: (id, request) =>
		api.post<void>(endpoints.platform.userMemberships(id), request),
	setPlatformMembershipRole: (id, tenantId, request) =>
		api.patch<void>(endpoints.platform.userMembership(id, tenantId), request),
	removePlatformMembership: (id, tenantId) =>
		api.delete<void>(endpoints.platform.userMembership(id, tenantId)),

	getDashboard: () => api.get<Dashboard>(endpoints.dashboard),
	listAlerts: (query) =>
		api.get<AlertListResponse>(endpoints.alerts.list, {
			query: (query ?? {}) as Record<string, string | number | undefined>
		}),
	listAlertRules: () =>
		api.get<{ items: AlertRule[] }>(endpoints.alerts.rules).then((response) => response.items),
	updateAlertRules: (request) =>
		api
			.put<{ items: AlertRule[] }>(endpoints.alerts.rules, request)
			.then((response) => response.items),
	listChannels: () => api.get<ChannelList>(endpoints.channels.list),
	createChannel: (request) => api.post<ChannelSaved>(endpoints.channels.create, request),
	updateChannel: (id, request) => api.patch<ChannelSaved>(endpoints.channels.byId(id), request),
	deleteChannel: (id) => api.delete<void>(endpoints.channels.byId(id)),
	testChannel: (id) => api.post<ChannelTestResult>(endpoints.channels.test(id)),

	listAudit: (query) =>
		api.get<AuditPage>(endpoints.audit.list, {
			query: query as Record<string, string | number | undefined>
		}),

	listPlatformAudit: (query) =>
		api.get<AuditPage>(endpoints.platform.audit, {
			query: query as Record<string, string | number | undefined>
		})
};

export const useMock = import.meta.env.VITE_USE_MOCK === 'true';

/**
 * The service of the application.
 *
 * The mock is loaded with a dynamic import and only when `VITE_USE_MOCK` is set.
 * A production build replaces the flag with `false` and removes the import, so
 * the mock code never reaches the bundle.
 */
export const service: ApiService = useMock
	? ((await import('./mock')).mockApi as unknown as ApiService)
	: httpService;
