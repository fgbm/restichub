# ADR-0013: Documentation: Starlight, STE, Vale

- Status: accepted
- Date: 2026-10-12
- Requirements: NFR-DOC-01 to NFR-DOC-04, NFR-OSS-02

## Context

The documentation must be clear for readers whose first language is not English. It must be available in English and Russian. CI must check the style.

ASD-STE100 Simplified Technical English (Issue 9, 2025) gives writing rules and a controlled dictionary. No equivalent standard exists for Russian.

## Decision

- The documentation site uses Starlight (Astro). It is in `docs/`. English is the default language. Russian is the second language.
- English text obeys ASD-STE100. The project summary of the rules is in `design/style/ste-en.md`.
- Russian text obeys the project rules in `design/style/ru-technical.md`. These rules follow the same principles as STE.
- Vale checks the text in CI with project rules in `styles/`. No ready Vale package for STE was found, so the project keeps its own rules.
- Design documents in `design/` are in English and obey the same rules.

## Consequences

- Short sentences and one term for one concept make translation easier.
- Vale rules catch only part of the STE rules. A review checklist covers the rest (AC-DOC-02).
- The STE dictionary is not in the repository. Writers use the official dictionary and the project glossary.

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| Docusaurus | Good versioning, but heavier. Starlight has built-in i18n and is faster. |
| mdBook | No good i18n support. |
| No style rules | Inconsistent and hard to translate text. |
