-- Local users and memberships (ADR-0023, section Local authentication).
--
-- A username is unique for each tenant and the comparison ignores the letter case (D6). Sign-in
-- takes the tenant name and the username.
-- The role `platform_admin` is valid in the platform tenant only (the constant
-- `PLATFORM_TENANT_ID` in the Rust code).

CREATE TABLE users (
    id uuid NOT NULL PRIMARY KEY,
    tenant_id uuid NOT NULL,
    username text NOT NULL,
    password_hash text NOT NULL,
    display_name text NOT NULL,
    disabled boolean NOT NULL DEFAULT false,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT users_tenant_id_fk FOREIGN KEY (tenant_id) REFERENCES tenants (id),
    CONSTRAINT users_tenant_id_id_unique UNIQUE (tenant_id, id)
);

-- The username is unique for the tenant and the comparison ignores the letter case (D6).
CREATE UNIQUE INDEX users_tenant_username_lower_unique ON users (tenant_id, lower(username));

CREATE TABLE memberships (
    tenant_id uuid NOT NULL,
    user_id uuid NOT NULL,
    role text NOT NULL,
    CONSTRAINT memberships_pkey PRIMARY KEY (tenant_id, user_id),
    CONSTRAINT memberships_role_check
        CHECK (role IN ('platform_admin', 'admin', 'operator', 'viewer')),
    CONSTRAINT memberships_tenant_id_fk FOREIGN KEY (tenant_id) REFERENCES tenants (id),
    CONSTRAINT memberships_user_fk FOREIGN KEY (tenant_id, user_id)
        REFERENCES users (tenant_id, id),
    CONSTRAINT memberships_platform_admin_only_platform
        CHECK (
            role <> 'platform_admin'
            OR tenant_id = '00000000-0000-7000-8000-000000000000'
        )
);

ALTER TABLE users ENABLE ROW LEVEL SECURITY;
ALTER TABLE users FORCE ROW LEVEL SECURITY;
CREATE POLICY users_tenant ON users
    FOR ALL
    USING (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid);

ALTER TABLE memberships ENABLE ROW LEVEL SECURITY;
ALTER TABLE memberships FORCE ROW LEVEL SECURITY;
CREATE POLICY memberships_tenant ON memberships
    FOR ALL
    USING (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid);
