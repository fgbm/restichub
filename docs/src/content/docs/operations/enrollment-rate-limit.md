---
title: Agent enrollment rate limit
description: The rate limit of the agent enrollment endpoint, behind a proxy and behind a NAT.
---

The control plane serves the agent enrollment on `POST /agent/v1/enroll`. This endpoint is the only endpoint without a client certificate (FR-ID-05). The control plane limits the request rate, so one client cannot flood the endpoint.

## The limit

- The control plane allows 10 enrollment requests for each source IP address in one minute.
- The control plane allows 10 enrollment requests for each enrollment token prefix in one minute.
- The 11th request over a limit gives HTTP 429 and a `Retry-After` header with the wait time in seconds.

The token prefix is the first eight characters of the token value after `re_`. One token has one counter. A client cannot avoid the limit with many request bodies, because the counter reads the prefix.

## Behind a reverse proxy

The control plane reads the client address from the socket peer. It reads the `X-Forwarded-For` header only when the socket peer is a trusted proxy. Set `RESTICHUB_TRUSTED_PROXIES` to the network of the reverse proxy. The value is a comma-separated list of networks in CIDR form.

An empty list makes the control plane ignore `X-Forwarded-For`. Every request from the proxy then holds the proxy address, so the rate limit counts the proxy.

## Behind a NAT

The limit counts the source IP address. Many hosts behind one NAT share one source IP address. They share one counter. A fleet of more than 10 hosts behind one NAT reaches the limit in one minute. Give each host its own enrollment token. The token prefix limit stays separate for each token.

## Result

- One client cannot flood the endpoint.
- A fleet behind one NAT or one proxy needs a plan for the shared address.
