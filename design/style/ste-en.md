# English style guide (STE)

English text in ResticHub obeys ASD-STE100 Simplified Technical English, Issue 9. This guide is a project summary. It does not replace the standard. Get the full standard and the dictionary from the [ASD-STE100 website](https://www.asd-ste100.org).

This guide applies to: documentation, design documents, UI text, error messages, log messages, commit messages.

This guide does not apply to code comments and doc comments. Those use plain short technical English; the rules are in [AGENTS.md](../../AGENTS.md), section Code rules.

## 1. Words

<!-- vale off -->

1. Use words from the STE dictionary with their approved meaning only.
2. Use technical nouns and technical verbs from the [project glossary](#5-project-glossary) and from the field (for example "repository", "snapshot", "certificate").
3. Use one word for one concept. Do not use synonyms. For example, always "agent", never "client" or "node" for the same thing.
4. Do not use phrasal verbs ("set up", "carry out"). Use one verb ("configure", "do").
5. Do not use contractions ("don't", "it's").
6. Do not use "please", "simply", "just", "easily", "obviously".
7. Use "must" for a requirement. Do not use "should" for a requirement.

<!-- vale on -->

## 2. Sentences

1. A sentence in a procedure has 20 words or fewer.
2. A sentence in a description has 25 words or fewer.
3. A paragraph has 6 sentences or fewer.
4. Use the active voice. Write "The agent sends the result", not "The result is sent".
5. Use the simple tenses: present, past, future.
6. Write one topic in each sentence.

## 3. Procedures

1. Write one instruction in each step.
2. Use the imperative: "Open the file".
3. If a condition applies, write the condition first: "If the agent is offline, do step 4".
4. Put a warning or a caution before the step that it applies to.
5. Number the steps.

## 4. Notes, warnings and cautions

| Type | Use |
|---|---|
| Warning | Risk of data loss or security problem. |
| Caution | Risk of damage to the configuration or the service. |
| Note | Useful information. No risk. |

Write a warning or a caution as a short command, then the reason: "Do not run `unlock --remove-all` while a job runs. Data loss can occur."

## 5. Project glossary

<!-- vale off -->

| Term | Meaning | Do not use |
|---|---|---|
| agent | The ResticHub program on a host | client, node, daemon |
| control plane | The ResticHub server | server (alone), backend, master |
| host | A computer with an agent | machine, box |
| tenant | An organization: the isolation unit that owns hosts, repositories, plans, profiles, secrets and memberships (ADR-0006, ADR-0024, ADR-0025) | customer, organization (for this concept) |
| repository | A restic repository | repo, storage |
| profile | A backup definition | config, preset |
| plan | A profile assigned to agents, with a schedule | task, schedule (for this concept) |
| job | One run of a plan or a manual operation | task, run (as a noun) |
| step | One command in a job | action, stage |
| hook | An allow-listed command before or after restic | script, trigger |
| allow-list | The list of permitted hooks on a host | whitelist |
| maintenance agent | The agent that runs `forget`, `prune`, `check` | |
| snapshot | A restic snapshot | backup (for this concept) |
| enroll, enrollment | The first registration of an agent | register, onboard |
| revoke | Make an agent certificate not valid | ban, block |

<!-- vale on -->

## 6. Review checklist

Use this checklist for each review of text (AC-DOC-02):

- [ ] Each sentence has the correct maximum length.
- [ ] Each procedure step has one instruction in the imperative.
- [ ] The text uses the active voice.
- [ ] Each concept has one term from the glossary.
- [ ] There are no phrasal verbs, contractions or filler words.
- [ ] Warnings and cautions come before the steps.
- [ ] Vale shows no errors.
