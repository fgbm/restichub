---
title: Organizations
description: Create, suspend, resume and delete an organization, and switch between organizations.
---

An organization holds its own hosts, repositories, profiles, plans, secrets and users. One organization cannot see the data of another organization. A Platform Admin manages the organizations of the installation.

:::caution
In the mode `single`, the installation serves one organization. Set `RESTICHUB_TENANCY_MODE=multi` before you create a second organization. In the mode `single`, the control plane does not start when the database holds more than one active organization.
:::

## Create an organization

1. Open the organization menu in the top bar, and select "Platform administration".
2. Open "Organizations".
3. Select "Create organization".
4. Enter the name. The name holds 1 to 63 characters from `a-z`, `0-9` and `-`. It starts with a letter and is not `platform`.
5. Select "Create organization".

The list shows the new organization with the mark "No administrator". The platform audit log holds the record `tenant_create`.

A new organization has no user. Add the first Admin on the page of a platform user. See [Users](/restichub/guides/users/#platform-users).

## Suspend and resume an organization

A suspended organization refuses the sign-in and the API tokens of its users. Its agents stay connected.

1. Open "Organizations".
2. In the menu "…" of the organization, select "Suspend".

To make the organization active again, select "Resume" in the same menu. The audit log holds the records `tenant_suspend` and `tenant_resume`.

## Delete an organization

:::danger
A delete removes the organization and all its data from the control plane. Data loss occurs. The data in the repositories stays.
:::

You can delete only a suspended organization.

1. Suspend the organization.
2. In the menu "…" of the organization, select "Delete".
3. Type the name of the organization to confirm.
4. Select "Delete organization".

The tab "Deleted" shows the organization. The audit log holds the record `tenant_delete`.

## Switch the organization

A person with a membership in more than one organization selects the organization in the top bar.

1. Select the name of the organization in the top bar.
2. Select another organization in the menu.

Each screen then shows the data of the new organization. The audit log holds the record `session_switch`.

## Related

- [Users](/restichub/guides/users/)
- [Environment variables](/restichub/reference/environment-variables/)
