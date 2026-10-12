# ADR-0012: Web UI: Svelte 5 single-page application in the control plane binary

- Status: accepted
- Date: 2026-10-12
- Requirements: FR-UI-01 to FR-UI-04, NFR-OPS-01

## Context

The web UI is an administration tool behind authentication. It does not need search engine optimization or server-side rendering. The installation must stay simple.

## Decision

- We use Svelte 5 with SvelteKit in single-page application mode (`ssr = false`, `adapter-static`).
- The control plane binary embeds the built files.
- The TypeScript API client is generated from the OpenAPI specification (ADR-0009).
- We pin the versions of SvelteKit and its plugins. SvelteKit 3.0 was released on 2026-10-06.
- The UI translates error codes from the API into English and Russian.

## Consequences

- One binary contains the backend and the UI.
- No Node.js process in production.
- The pool of Svelte contributors is smaller than the pool of React contributors.

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| Vue 3 | A good option. The maintainers prefer Svelte. |
| React | Larger ecosystem, but more boilerplate. |
| Leptos (Rust and WebAssembly) | Small ecosystem, long builds, few frontend contributors know Rust. |
| Separate UI server | An extra process in production. |
