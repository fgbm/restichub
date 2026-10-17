-- Plan, profile and alert channel names become free text (FR-PF-01). The name is unique for the
-- tenant and the comparison ignores the letter case, as for a username (D6). The old keys compared
-- the exact text, so they would allow "Nightly" next to "nightly".
ALTER TABLE profiles DROP CONSTRAINT profiles_tenant_name_unique;
CREATE UNIQUE INDEX profiles_tenant_name_lower_unique ON profiles (tenant_id, lower(name));

ALTER TABLE plans DROP CONSTRAINT plans_tenant_name_unique;
CREATE UNIQUE INDEX plans_tenant_name_lower_unique ON plans (tenant_id, lower(name));

ALTER TABLE notification_channels DROP CONSTRAINT notification_channels_name_unique;
CREATE UNIQUE INDEX notification_channels_tenant_name_lower_unique
    ON notification_channels (tenant_id, lower(name));
