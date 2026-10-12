-- ResticHub role bootstrap (D1, N3).
--
-- Run this script once for each PostgreSQL cluster, as a database administrator, before the first
-- migration of a ResticHub installation. The script creates the group role `restichub_app`, the
-- privileged definer role `restichub_auth` and the two login roles of the installation.
--
-- The script creates the logins without a password. Set each password after the script with the
-- psql meta-command `\password <role>`. That command hashes the password on the client (SCRAM).
-- Do not pass a password on the command line.
--
-- One ResticHub installation per PostgreSQL cluster: the group role `restichub_app` and the
-- definer role `restichub_auth` are shared by every database of one installation. Do not put two
-- installations in one cluster with this role model (M5).
--
-- The database administrator also creates the installation database and makes the owner login the
-- owner of that database. The owner login runs `restichub-cp migrate`. The runtime login serves the
-- control plane server pool.
--
-- Usage. Give the role names only:
--
--   psql -v app_role=restichub_app \
--        -v auth_role=restichub_auth \
--        -v owner_role=restichub_owner \
--        -v runtime_role=restichub_runtime \
--        --set ON_ERROR_STOP=on \
--        -f bootstrap-roles.sql
--
-- Then set the two passwords in an interactive psql session:
--
--   \password restichub_owner
--   \password restichub_runtime
--
-- The script is idempotent. It fails with a clear message if a pre-existing role has an unsafe
-- attribute.

\set ON_ERROR_STOP on

-- The group role of the runtime. It is not the table owner and has no BYPASSRLS.
SELECT format(
    'CREATE ROLE %I NOLOGIN NOSUPERUSER NOBYPASSRLS NOCREATEROLE NOCREATEDB',
    :'app_role'
)
WHERE NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = :'app_role')
\gexec

SELECT coalesce(
    (SELECT rolsuper OR rolbypassrls OR rolcreaterole OR rolcreatedb
     FROM pg_roles WHERE rolname = :'app_role'),
    false
) AS app_unsafe \gset
\if :app_unsafe
\echo 'error: the role' :app_role 'exists with a forbidden attribute (SUPERUSER, BYPASSRLS, CREATEROLE or CREATEDB)'
DO $$ BEGIN RAISE EXCEPTION 'bootstrap-roles.sql refused a role with a forbidden attribute'; END $$;
\endif

-- The definer role of the authentication lookups. It has BYPASSRLS, because FORCE ROW LEVEL
-- SECURITY also applies to the table owner. It cannot log in.
SELECT format(
    'CREATE ROLE %I NOLOGIN NOSUPERUSER BYPASSRLS NOCREATEROLE NOCREATEDB',
    :'auth_role'
)
WHERE NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = :'auth_role')
\gexec

SELECT coalesce(
    (SELECT rolcanlogin OR rolsuper OR NOT rolbypassrls
     FROM pg_roles WHERE rolname = :'auth_role'),
    false
) AS auth_unsafe \gset
\if :auth_unsafe
\echo 'error: the role' :auth_role 'must be NOLOGIN, NOSUPERUSER and BYPASSRLS'
DO $$ BEGIN RAISE EXCEPTION 'bootstrap-roles.sql refused a role with a forbidden attribute'; END $$;
\endif

-- The owner login. It owns the installation database and runs `restichub-cp migrate`.
SELECT format(
    'CREATE ROLE %I LOGIN NOSUPERUSER NOBYPASSRLS NOCREATEROLE NOCREATEDB',
    :'owner_role'
)
WHERE NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = :'owner_role')
\gexec

SELECT coalesce(
    (SELECT NOT rolcanlogin OR rolsuper OR rolbypassrls
     FROM pg_roles WHERE rolname = :'owner_role'),
    false
) AS owner_unsafe \gset
\if :owner_unsafe
\echo 'error: the role' :owner_role 'must be LOGIN, NOSUPERUSER and NOBYPASSRLS'
DO $$ BEGIN RAISE EXCEPTION 'bootstrap-roles.sql refused a role with a forbidden attribute'; END $$;
\endif

-- The runtime login. It serves the control plane server pool. It owns no object.
SELECT format(
    'CREATE ROLE %I LOGIN NOSUPERUSER NOBYPASSRLS NOCREATEROLE NOCREATEDB',
    :'runtime_role'
)
WHERE NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = :'runtime_role')
\gexec

SELECT coalesce(
    (SELECT NOT rolcanlogin OR rolsuper OR rolbypassrls
     FROM pg_roles WHERE rolname = :'runtime_role'),
    false
) AS runtime_unsafe \gset
\if :runtime_unsafe
\echo 'error: the role' :runtime_role 'must be LOGIN, NOSUPERUSER and NOBYPASSRLS'
DO $$ BEGIN RAISE EXCEPTION 'bootstrap-roles.sql refused a role with a forbidden attribute'; END $$;
\endif

-- Both logins are members of the group role. The owner login is also a member of the definer role,
-- so the migration can give the authentication functions to `restichub_auth`.
GRANT :"app_role" TO :"owner_role";
GRANT :"app_role" TO :"runtime_role";
GRANT :"auth_role" TO :"owner_role";

\echo 'The roles are ready. Set the passwords now with these commands:'
\echo '  \\password' :owner_role
\echo '  \\password' :runtime_role
