# English style guide (STE)

English text in ResticHub obeys ASD-STE100 Simplified Technical English, Issue 9. This guide is a project summary. It does not replace the standard. Get the full standard and the dictionary from the [ASD-STE100 website](https://www.asd-ste100.org).

This guide applies to documentation, design documents, UI text, error messages, log messages and commit messages.

## 1. Two levels

ResticHub uses STE at two levels.

| Text type | Rules |
|---|---|
| Procedure, step, warning, caution, UI text, error message, log message, commit message | This guide in full, with the STE dictionary. |
| Concept and Reference pages | Sections 2 to 6 without the dictionary. Use the technical words of the field and the project glossary. |

The dictionary has about 900 words. It cannot describe a certificate rotation or a cron expression. For this reason, explanation uses the technical vocabulary of the field. The other rules stay: short sentences, the active voice, one term for one concept and the readability targets.

Do not use the relaxed vocabulary as a reason for a synonym or a long sentence.

## 2. Words

<!-- vale off -->

1. Use words from the STE dictionary with their approved meaning only. In explanation, use common technical English instead.
2. Use technical nouns and technical verbs from the [project glossary](#7-project-glossary) and from the field (for example "repository", "snapshot", "certificate").
3. Use one word for one concept. Do not use synonyms. For example, always "agent", never "client" or "node" for the same thing.
4. The command `restichub-agent register` and the UI action "Re-register" keep their names. In the text, the concept is "enrollment".
5. Do not use phrasal verbs ("set up", "carry out"). Use one verb ("configure", "do").
6. Do not use contractions ("don't", "it's").
7. Do not use "please", "simply", "just", "easily", "obviously".
8. Use "must" for a requirement. Do not use "should" for a requirement.

<!-- vale on -->

## 3. Sentences

1. A sentence in a procedure has 20 words or fewer.
2. A sentence in a description has 25 words or fewer.
3. A paragraph has 6 sentences or fewer.
4. Use the active voice. Write "The agent sends the result", not "The result is sent".
5. A passive sentence can describe a state when the actor is unknown or does not matter: "The token is bound to the host." Do not use the passive for an action: write "The agent cancels the job", not "The job is cancelled".
6. Use the simple tenses: present, past, future.
7. Write one topic in each sentence.
8. Write one statement in each sentence. Do not join two independent statements with "and" or a comma.
9. Write one instruction or one statement in each list item. Two fields of one form section can be one instruction. Two independent facts need two items.

The limit of 25 words does not replace rules 8 and 9. A sentence of 20 words can hold three facts.

## 4. Procedures

1. Write one instruction in each step. A step can name two fields of one form section when they belong to one action.
2. Use the imperative: "Open the file".
3. If a condition applies, write the condition first: "If the agent is offline, do step 4".
4. Put a warning or a caution before the step that it applies to.
5. Number the steps.

## 5. Notes, warnings and cautions

| Type | Use |
|---|---|
| Warning | Risk of data loss or security problem. |
| Caution | Risk of damage to the configuration or the service. |
| Note | Useful information. No risk. |

Write a warning or a caution as a short command, then the reason: "Do not run `unlock --remove-all` while a job runs. Data loss can occur."

## 6. Readability targets

| Measure | Target |
|---|---|
| Average sentence length in prose | 15 words or fewer |
| Longest sentence | 25 words or fewer |
| Flesch Reading Ease | 55 or more |

Measure prose only. Do not count code, tables, list items, headings, UI labels and frontmatter.

Vale enforces the limit of each sentence. The average and the reading ease describe the page as a whole. A page outside a target is a candidate for shorter paragraphs and fewer facts in each sentence. It is not an automatic defect: a table of error codes or a list of fields can need long items.

## 7. Project glossary

<!-- vale off -->

| Term | Meaning | Do not use |
|---|---|---|
| agent | The ResticHub program on a host | client, node, daemon |
| control plane | The ResticHub server | server (alone), backend, master |
| host | A computer with an agent | machine, box |
| tenant | An organization: the isolation unit that owns hosts, repositories, plans, profiles, secrets and memberships (ADR-0006, ADR-0024, ADR-0025) | customer, organization (for this concept) |
| repository | A restic repository | repo, storage |
| profile | A backup definition: what to back up and how | config, preset |
| plan | A profile assigned to a host, with a repository and a schedule | task, schedule (for this concept) |
| profile parameter | A value that a profile declares for its plans; a host or a plan sets the value | variable, argument, setting (for this concept) |
| parameter value | The value of a profile parameter on a host or on a plan | variable value, argument value, setting (for this concept) |
| job | One run of a plan or a manual operation | task, run (as a noun) |
| step | One command in a job | action, stage |
| script | A command text that runs before or after restic on a host with the trust mode | hook, trigger |
| maintenance agent | The agent that runs `forget`, `prune`, `check` | |
| snapshot | A restic snapshot | backup (for this concept) |
| enroll, enrollment | The first registration of an agent | register, onboard |
| revoke | Make an agent certificate not valid | ban, block |

<!-- vale on -->

## 8. Review checklist

Use this checklist for each review of text (AC-DOC-02):

- [ ] Each sentence has the correct maximum length.
- [ ] Each sentence and each list item holds one statement.
- [ ] Each procedure step has one instruction in the imperative.
- [ ] The text uses the active voice. A passive sentence describes only a state.
- [ ] Each concept has one term from the glossary.
- [ ] There are no phrasal verbs, contractions or filler words.
- [ ] Warnings and cautions come before the steps.
- [ ] The page meets the readability targets, or the reason for a miss is clear.
- [ ] Vale shows no errors.
