---
title: API reference
description: How to sign in to the API, which headers it needs, and how to read the OpenAPI document.
---

The control plane serves a REST API under `/api/v1`. The web UI uses the same API. This page shows how to authenticate. The OpenAPI document lists every route, field and error.

## OpenAPI document

The control plane serves the OpenAPI document at `GET /api/v1/openapi.json`. The route needs no credential.

```sh
curl -s https://<control-plane>/api/v1/openapi.json -o restichub-openapi.json
```

Load the file in an OpenAPI viewer or in a code generator. The document describes two security schemes: `cookie` for a session and `bearer` for an API token.

## Sign in with an API token

Use an API token for a script. A token belongs to one tenant and has one role.

1. Open "API tokens" in the web UI.
2. Select "Create token". Enter a name, a role and an optional expiry time.
3. Copy the token value. The control plane shows it one time only.

A token value starts with `rh_`. Send it in the `Authorization` header:

```sh
curl -s https://<control-plane>/api/v1/plans \
  -H "Authorization: Bearer rh_<token>"
```

A request with a token needs no CSRF header. The role of the token limits the routes. A route that the role does not allow gives HTTP 403.

Create a token in the API with a session or with a token that has the Admin role:

```sh
curl -s -X POST https://<control-plane>/api/v1/api-tokens \
  -H "Authorization: Bearer rh_<admin-token>" \
  -H "Content-Type: application/json" \
  -d '{"name": "ci", "role": "viewer", "expires_at": "2027-01-01T00:00:00Z"}'
```

The response holds the field `token`. The control plane never returns it again.

## Sign in with a session

The web UI uses a session cookie. A script can do the same:

```sh
curl -s -c cookies.txt -X POST https://<control-plane>/api/v1/auth/sign-in \
  -H "X-ResticHub-CSRF: 1" \
  -H "Content-Type: application/json" \
  -d '{"username": "alice", "password": "<password>"}'
```

The control plane sets the cookie `__Host-restichub_session`. A request that changes data with this cookie must also send the header `X-ResticHub-CSRF: 1`. The sign-in request needs the header too. A request without the header gives HTTP 403.

```sh
curl -s -b cookies.txt -X POST https://<control-plane>/api/v1/plans/<plan-id>/run \
  -H "X-ResticHub-CSRF: 1"
```

A request must not hold a cookie and a bearer token together. That request gives HTTP 400.

## Errors

An error response uses the media type of RFC 9457. The body holds these fields:

| Field | Meaning |
|---|---|
| `status` | The HTTP status. |
| `code` | A stable code in snake_case. The web UI translates it. |
| `title` | A short English text. |
| `request_id` | The identifier of the request. Give it to the operator of the control plane. |
| `field`, `reason` | The wrong field and the reason. Only the code `invalid_argument` sets them. |

## Lists

A list route returns `items` and `next_cursor`. To read the next page, send the value of `next_cursor` as the parameter `cursor`. The parameter `limit` sets the page size.

The lists of hosts, jobs, alerts and audit records also read a numbered page. Send the parameter `offset` with the number of records to skip. The answer then has `total`, the number of records that agree with the filters. The control plane counts at most 10000 records. If more records agree, `total_capped` is `true`. An `offset` above 10000 gives HTTP 400 `invalid_argument` with the field `offset` and the reason `out_of_range`. Do not send `cursor` and `offset` together.

The lists of jobs and alerts also give `counts`: the number of records of each state tab with the other filters. Each tab has `count` (at most 10000) and `capped`. The lists of hosts, jobs, alerts and audit records find text with the parameter `q`, at most 200 characters. The characters `%`, `_` and `\` match themselves. In the audit list, `q` finds a part of the action code, of the object name or of the actor name.

An audit record gives `object_name` and `actor_name`: the current names of the object and of the actor. The value is null when the object is removed or has no name. In the platform list, the value is null also for an object inside an organization.

```sh
curl -s "https://<control-plane>/api/v1/plans?limit=50" \
  -H "Authorization: Bearer rh_<token>"
```

## Live events

The route `GET /api/v1/events` sends Server-Sent Events. The web UI opens one stream for each session. The stream tells the UI when to read the data again. This way the UI does not poll.

```sh
curl -N "https://<control-plane>/api/v1/events" \
  -H "Authorization: Bearer rh_<token>"
```

The stream shows only the events of your organization. Each event needs a read right. The role Viewer has all of them.

| Event | Meaning |
|---|---|
| `resync` | The first event of each stream. Read the data again. |
| `job.progress` | A step of a job made progress, or a step ended. |
| `job.status` | A job changed its status. The field `status` holds the new status. |
| `job.log` | The log of a job got new lines. |
| `host.online`, `host.offline` | A host connected or left. The field `agent_id` names the host. |
| `alert.changed` | An alert opened or resolved. |

An event holds identifiers and a status only. It holds no secret and no log line. Read the REST API for the data.

The control plane keeps no event history. After a reconnect, the stream sends `resync` first. Ignore `Last-Event-ID` and read the data again. A comment line every 15 seconds keeps the connection open. A stream ends after at most 15 minutes. It ends sooner when the access ends. Examples are a sign-out, a disabled user, a changed role, a suspended organization and a revoked API token. The control plane checks this every 30 seconds. A stream also ends when the server stops, and when the client does not read it for 10 seconds. The client then connects again. One session or one API token can hold 4 streams, one address 100 streams, one organization 200 streams, and the server 2000 streams (`RESTICHUB_EVENTS_MAX_STREAMS`). A request above a limit gets HTTP 429.

A session can hold 4 streams at the same time, and an organization can hold 200. More requests get HTTP 429.

A reverse proxy must not buffer the stream. The response sets `X-Accel-Buffering: no` for nginx. For another proxy, turn off the response buffering for this route and allow a read timeout above 30 seconds.

## Related pages

- [Prometheus metrics](/restichub/operations/metrics/)
- [Environment variables](/restichub/operations/environment-variables/)
