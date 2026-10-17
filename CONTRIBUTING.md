# Contributing to ResticHub

Thank you for your help. This document tells you how to make a change.

## Before you start

1. Read the [requirements](design/requirements.md) and the [architecture decision records](design/adr/).
2. For a large change, open an issue first. Describe the problem and your proposal.
3. If your change alters an accepted decision, write a new ADR. Use the [template](design/adr/0000-template.md).

## Developer Certificate of Origin

All commits must have a `Signed-off-by` line. This line tells that you have the right to submit the change under the project license. Use `git commit -s`. The project does not use a Contributor License Agreement.

## Commit messages

Use [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/):

```text
feat(agent): cancel restic with Ctrl+Break on Windows
fix(scheduler): skip a fire time in a DST gap only once
docs(design): add ADR for log storage
```

## Code

- Run `cargo fmt --all` before you commit.
- Make sure that `cargo clippy --workspace --all-targets -- -D warnings` shows no warnings.
- Add tests for each change in behavior. Each acceptance criterion in [`design/acceptance-criteria.md`](design/acceptance-criteria.md) must have a test that refers to its ID.
- Do not log secrets. Do not put secrets in process arguments.

## Documentation

- Write English documentation in ASD-STE100 Simplified Technical English. Read [`design/style/ste-en.md`](design/style/ste-en.md).
- Write Russian documentation by the rules in [`design/style/ru-technical.md`](design/style/ru-technical.md).
- Vale checks the documentation in CI. Fix all errors before you ask for a review.
- Write a diagram in Mermaid in `docs/src/diagrams/<name>.<en|ru>.mmd`, and show it with the `Diagram` component in an `.mdx` page. The build makes an SVG that follows the site theme. Write `{{message.key}}` to show a label of the web interface.

## Pull requests

1. Keep each pull request on one topic.
2. Fill in the pull request template.
3. Make sure that CI passes.
4. A maintainer must approve the pull request before the merge.
