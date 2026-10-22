---
title: Automatic enrollment of hosts
description: Enroll many hosts from a script, Ansible or cloud-init with the API and one enrollment token for each host.
---

A script can enroll hosts without the web UI. A control machine gets an enrollment token from the API. The host receives the token and runs `register` from the folder of the package. The command installs the agent, enrolls the host and starts the service. This page shows the procedure and examples for a shell script, Ansible, cloud-init and PowerShell.

The examples use `https://backup.example.com` as the control plane URL. Replace it with your URL.

## Prerequisites

- An API token with the Admin role. Create it in "Settings" → "Access" → "API tokens" (see [API tokens](/restichub/guides/api-tokens/) and [API reference](/restichub/reference/api/#sign-in-with-an-api-token)).
- A verified agent package for each operating system of your hosts (see [Verify a release](/restichub/operations/verify-a-release/)).
- `root` on a Linux host, or an account of the local Administrators group on a Windows host.

:::caution
The Admin role can do all operations in the organization. Keep the API token in a secret store. Give it a short expiry. The project plans a narrower role that can only create enrollment tokens ([issue #229](https://github.com/fgbm/restichub/issues/229)).
:::

## Step 1: get an enrollment token

Run this request on the control machine. Each request gives one token:

```sh
printf 'Authorization: Bearer %s\n' "$RESTICHUB_API_TOKEN" |
  curl -fsS -H @- -X POST https://backup.example.com/api/v1/enrollment-tokens \
    -H 'Content-Type: application/json' \
    -d '{"labels": ["linux", "web"], "ttl_seconds": 900}'
```

The header comes from the standard input, so the API token is not in the process arguments of `curl`.

| Field | Value |
|---|---|
| `labels` | Optional. The control plane gives these labels to the host that uses the token. |
| `ttl_seconds` | Optional. The time to live of the token, from 60 to 86400 seconds. The default is 3600. |

The response has the HTTP status 201. It holds the token value in the field `token`, for example `re_<token>`. The control plane shows the value one time only. Keep the field `id` to verify the enrollment later.

A token enrolls one host one time. Get a new token for each host. Give the token a time to live that is a little longer than the install of one host.

## Step 2: register the host

1. Download the agent package to the host.
2. Compare the SHA-256 of the package with the verified `SHA256SUMS` of the release.
3. Unpack the package in any folder.
4. Run `register --url https://backup.example.com --token-stdin` from the folder of the package, as `root` or Administrator.
5. Give the token on the standard input of the command.

Use `--token-stdin` or `--token-file PATH` in a script. Do not use `--token`, because other accounts can read the process arguments.

The command does these steps:

- It checks the rights and the directories. A failed check does not use the token.
- It copies the agent into `/opt/restichub-agent` on Linux, or into `%ProgramFiles%\ResticHub\Agent` on Windows.
- It downloads and verifies restic if the host has none.
- It enrolls the host and writes the identity into `/etc/restichub-agent` or `%ProgramData%\ResticHub\Agent`.
- It installs and starts the service `restichub-agent`: a systemd unit on Linux, a Windows service on Windows.
- It waits up to 60 seconds for the connection and prints a summary.

The exit code is 0 only when the agent connects to the control plane. See [Add a host](/restichub/getting-started/add-a-host/#the-registration-command) for all the options and the summary.

## Examples

### Shell script for one host

The script runs on a control machine with `curl`, `jq` and `ssh`. It copies a verified Linux package to the host, gets a token and registers the host:

```sh
#!/bin/sh
# Usage: enroll.sh <host> <package.tar.gz>
set -eu
CP_URL=https://backup.example.com
HOST=$1
PACKAGE=$2

scp "$PACKAGE" "root@$HOST:/tmp/restichub-agent.tar.gz"
ssh "root@$HOST" 'mkdir -p /tmp/restichub-agent &&
  tar -xzf /tmp/restichub-agent.tar.gz -C /tmp/restichub-agent'

printf 'Authorization: Bearer %s\n' "$RESTICHUB_API_TOKEN" |
  curl -fsS -H @- -X POST "$CP_URL/api/v1/enrollment-tokens" \
    -H 'Content-Type: application/json' \
    -d '{"labels": ["linux"], "ttl_seconds": 900}' |
  jq -r .token |
  ssh "root@$HOST" "/tmp/restichub-agent/restichub-agent register --url $CP_URL --token-stdin"
```

Without `jq`, replace `jq -r .token` with `sed -n 's/.*"token":"\([^"]*\)".*/\1/p'`.

### Ansible

The first task gets the token on the control machine. `no_log: true` keeps the API token and the enrollment token out of the output. The tasks skip a host that has an identity already:

```yaml
- name: Find the agent identity
  ansible.builtin.stat:
    path: /etc/restichub-agent/identity.json
  register: restichub_identity

- name: Create an enrollment token
  ansible.builtin.uri:
    url: "{{ restichub_url }}/api/v1/enrollment-tokens"
    method: POST
    headers:
      Authorization: "Bearer {{ restichub_api_token }}"
    body_format: json
    body:
      labels: ["linux", "web"]
      ttl_seconds: 900
    status_code: 201
  delegate_to: localhost
  become: false
  no_log: true
  register: restichub_enrollment
  when: not restichub_identity.stat.exists

- name: Register the agent
  ansible.builtin.command:
    cmd: "/tmp/restichub-agent/restichub-agent register --url {{ restichub_url }} --token-stdin"
    stdin: "{{ restichub_enrollment.json.token }}"
  become: true
  no_log: true
  when: not restichub_identity.stat.exists
```

For a Windows host, use `ansible.windows.win_stat` with the path `C:\ProgramData\ResticHub\Agent\identity.json`. Then use `ansible.windows.win_command` with the same `stdin` argument.

### cloud-init

Get a token for each new virtual machine and put it in the user data. Put the SHA-256 of the package from the verified `SHA256SUMS` in the user data too:

```yaml
#cloud-config
write_files:
  - path: /run/restichub/enroll-token
    permissions: "0600"
    content: re_<token>
runcmd:
  - - sh
    - -c
    - |
      set -eu
      VERSION=0.1.0
      PACKAGE=restichub-agent-$VERSION-linux-amd64.tar.gz
      cd /tmp
      curl -fsSLO "https://github.com/fgbm/restichub/releases/download/v$VERSION/$PACKAGE"
      echo "<sha256>  $PACKAGE" | sha256sum -c -
      mkdir -p restichub-agent
      tar -xzf "$PACKAGE" -C restichub-agent
      restichub-agent/restichub-agent register --url https://backup.example.com \
        --token-file /run/restichub/enroll-token
      rm -f /run/restichub/enroll-token
```

The user data stays on the virtual machine and in the cloud platform. The token is single use and has a short time to live, so a copy of the used token cannot enroll a host.

### PowerShell for Windows

1. On the host, download and unpack the package in an elevated PowerShell:

   ```powershell
   $Version = '0.1.0'
   $Zip = "restichub-agent-$Version-windows-amd64.zip"
   Invoke-WebRequest "https://github.com/fgbm/restichub/releases/download/v$Version/$Zip" -OutFile "C:\Temp\$Zip"
   $Expected = '<sha256 from the verified SHA256SUMS>'
   if ((Get-FileHash "C:\Temp\$Zip" -Algorithm SHA256).Hash -ne $Expected) { throw 'The checksum is not correct' }
   Expand-Archive "C:\Temp\$Zip" -DestinationPath 'C:\Temp\restichub-agent'
   ```

2. On the control machine, get a token and register the host with PowerShell remoting:

   ```powershell
   $Url = 'https://backup.example.com'
   $Headers = @{ Authorization = "Bearer $env:RESTICHUB_API_TOKEN" }
   $Body = @{ labels = @('windows'); ttl_seconds = 900 } | ConvertTo-Json
   $Created = Invoke-RestMethod -Method Post -Uri "$Url/api/v1/enrollment-tokens" `
     -Headers $Headers -ContentType 'application/json' -Body $Body
   Invoke-Command -ComputerName host01 -ArgumentList $Created.token, $Url -ScriptBlock {
     param($Token, $Url)
     $Token | & 'C:\Temp\restichub-agent\restichub-agent.exe' register --url $Url --token-stdin
   }
   ```

The token goes to the host as an argument of the script block, not as a process argument of the agent.

## Verify the enrollment

Read the token by its `id`:

```sh
printf 'Authorization: Bearer %s\n' "$RESTICHUB_API_TOKEN" |
  curl -fsS -H @- "https://backup.example.com/api/v1/enrollment-tokens/<id>" |
  jq '{used_at, used_by_agent_id}'
```

The field `used_by_agent_id` holds the identifier of the host that used the token. It is null until a host uses the token.

To list the hosts with a label, use the hosts list:

```sh
printf 'Authorization: Bearer %s\n' "$RESTICHUB_API_TOKEN" |
  curl -fsS -H @- "https://backup.example.com/api/v1/agents?label=web&state=online,waiting" |
  jq -r '.items[] | "\(.id) \(.hostname)"'
```

A new host has the state `waiting` until the agent opens its stream. Then it has the state `online`.

## Enroll an existing host again

A host keeps its identifier, plans and labels when it enrolls again (see [Enroll a host again](/restichub/guides/agent-certificates/)).

1. Send `POST /api/v1/agents/<id>/reenroll` with the body `{}` or `{"ttl_seconds": 600}`.
2. Read the token value from the field `token` of the response.
3. On the host, run `register --force --url https://backup.example.com --token-stdin` from the folder of the package.

```sh
printf 'Authorization: Bearer %s\n' "$RESTICHUB_API_TOKEN" |
  curl -fsS -H @- -X POST "https://backup.example.com/api/v1/agents/<id>/reenroll" \
    -H 'Content-Type: application/json' -d '{}' |
  jq -r .token
```

The response has the same fields as in step 1, and the field `replaces_agent_id` holds the host identifier. The token takes no labels. Its time to live starts at 60 seconds. The limit and the default come from [`RESTICHUB_REENROLL_TOKEN_MAX_TTL_SECS`](/restichub/reference/environment-variables/): 3600 seconds by default, 86400 at most. A new token for a host cancels the earlier unused tokens of that host. A revoked host gives HTTP 409.

## Limits

- The enrollment endpoint allows 10 requests in one minute from one IP address, and 10 requests for one token. See [Agent enrollment rate limit](/restichub/operations/enrollment-rate-limit/).
- Behind a reverse proxy, set [`RESTICHUB_TRUSTED_PROXIES`](/restichub/reference/environment-variables/). Otherwise all hosts share the counter of the proxy address.
- Many hosts behind one NAT share one counter. Enroll more than 10 such hosts in groups, with one minute between the groups.
- A token expires after its time to live: 60 to 86400 seconds.
- A label has 1 to 63 characters: `a-z`, `0-9` and `-`. The control plane removes duplicate labels.

## Troubleshooting

| Symptom | Cause | Action |
|---|---|---|
| `register` stops: the control plane refused the enrollment | A host already used the token, the token expired, or an Admin revoked it. | Get a new token. One token enrolls one host. |
| `register` gets HTTP 429 | Too many enrollment requests from one address or for one token. | Wait for the time in `Retry-After`, then run `register` again. Enroll fewer hosts at a time. |
| The request for a token gets HTTP 403 | The API token does not have the Admin role. | Use an API token with the Admin role. |
| The request for a token gets HTTP 400 | A label or `ttl_seconds` is not valid. | Read the field name in the error. See [Limits](#limits). |
| `register` cannot connect to the URL | The URL is wrong, or the scheme is not `https`. | Use the URL of the web UI, for example `https://backup.example.com`. |
| `register` reports a certificate error | A private CA signed the certificate of the control plane. | Add `--ca-file PATH` with the certificate of that CA. |
| The command waits for the standard input | The token did not come on the standard input. | Give the token with a pipe, or use `--token-file PATH`. |

See also [Troubleshooting](/restichub/reference/troubleshooting/#hosts).

## Related

- [Add a host](/restichub/getting-started/add-a-host/)
- [Agent certificates and enrollment](/restichub/guides/agent-certificates/)
- [API tokens](/restichub/guides/api-tokens/)
- [Agent packages](/restichub/reference/agent-packages/)
- [Agent commands and files](/restichub/reference/agent-commands/)
