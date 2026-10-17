---
title: Users
description: How to manage the accounts, the memberships and the passwords of the users.
---

An account is installation-wide. A user signs in with the username and the password. The account has a membership in one or more organizations, and each membership has one role. The Platform Admin right is a right of the account, not a role in an organization.

## Organization users

An Admin of an organization manages the users of that organization.

1. Open "Settings", then "Users".
2. Select the name of a user. The page of the user opens.
3. Select a role in the "Access" section. The change applies immediately. The text below the field tells what the role permits.

The "…" menu of the page has "Disable user". The membership becomes disabled, and the user cannot work in this organization. The account stays. Select "Enable user" to give the access again.

The "…" menu of the list has "Change role", "Reset password" and "Disable user".

Some actions are not available:

- An Admin cannot change its own role, cannot disable itself and cannot reset its own password.
- An Admin cannot reset the password of a shared account. A shared account has a membership in another organization. The user or a Platform Admin resets that password.
- An Admin cannot change an account with the Platform Admin right. Only a Platform Admin manages it.

The screen shows the reason next to each action that is not available.

## Platform users

A Platform Admin manages every account of the installation.

1. Open "Platform administration", then "Users".
2. Select the name of a user. The page of the user opens.

The "Memberships" table shows the organizations of the user:

- To change a role, select a role in the row. The change applies immediately. If the change fails, the field shows the role that the control plane keeps.
- To remove a membership, select the delete icon and confirm.
- To add a membership, select an organization and a role below the table, then select "Add an organization". The list shows only the active organizations where the user has no membership.

A membership can show "Disabled" or "Suspended". "Disabled" means that an Admin of that organization disabled the membership. "Suspended" means that the organization is suspended.

The Platform Admin right gives no access to the data of an organization. To work in an organization, add a membership for your own account.

## Reset a password

1. Open the page of the user.
2. Select "Reset password".
3. Enter a new password with 12 characters or more.
4. Select "Reset password".

The old password stops, and every session of the account ends. The platform audit and the audit of each organization of the account record the reset without the password.

The reset does not revoke the API tokens that the account created. To stop all access of the account, revoke these tokens separately.

A Platform Admin can reset the password of another Platform Admin.

A Platform Admin cannot reset its own password on this page. To change your own password, send `POST /api/v1/auth/password` with the current password.

## Grant and remove the Platform Admin right

1. Open the page of the user.
2. Open the "…" menu and select "Grant the Platform Admin right".
3. Enter a new password for the account and confirm.

The grant needs a new password, because an Admin of an organization can know the old password. The old password and every session of the account stop.

To remove the right, select "Remove the Platform Admin right" in the "…" menu and confirm. The memberships of the account do not change.

You cannot remove your own right. You cannot remove the right of the last active Platform Admin.

## Disable an account

1. Open the page of the user.
2. Open the "…" menu and select "Disable user".
3. Confirm.

The user cannot sign in to any organization. Every session of the account ends. The memberships and the roles stay. Select "Enable user" to give the access again.

You cannot disable your own account. You cannot disable the last active Platform Admin.

## Related pages

- [API reference](/restichub/reference/api/)
- [Security for operators](/restichub/operations/security-for-operators/)
