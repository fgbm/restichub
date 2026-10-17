---
title: API tokens
description: Create an API token for an external tool, use it, and revoke it.
---

An API token lets an external tool use the API without a session. A token belongs to one organization and has one role. Only an Admin manages the tokens.

## Create a token

1. Open "Settings" → "Access" → "API tokens".
2. Select "Create token".
3. Enter a name, for example the name of the tool.
4. Select a role. Give the smallest role that the tool needs.
5. Select an expiry time: "Never", "In 30 days", "In 90 days" or "In 1 year".
6. Select "Create token".
7. Copy the token value.

:::caution
The control plane shows the value one time only. Keep it in a secret store of the tool. If you lose the value, revoke the token and create a new token.
:::

The list shows the token with its last use and its expiry. Select a token to see its prefix, role, state and dates. The tab "Expired and revoked" shows the tokens that no longer work.

## Use a token

Send the value in the header `Authorization: Bearer rh_<token>`. See [API reference](/restichub/reference/api/#sign-in-with-an-api-token).

## Revoke a token

Revoke a token when a tool no longer needs it, or when another person can know the value.

1. Open "API tokens".
2. In the menu "…" of the token, select "Revoke".
3. Select "Revoke" to confirm.

The tools with this token stop at once. The audit log holds the record `api_token_revoke`.

## Related

- [API reference](/restichub/reference/api/)
- [Users](/restichub/guides/users/)
