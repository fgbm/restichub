#!/usr/bin/env python3
"""Classify the files that a pull request changes, so CI runs only the jobs that can fail.

Usage: ci-changes.py [--all] [--base REF]
Prints one `name=true|false` line for each job group, ready for $GITHUB_OUTPUT, and an
`agent-packages` line with the `-p` arguments that select the agent crates for the Windows job.
--all marks every group true (a push to main, a tag, a manual run). Without --all, the script
diffs REF..HEAD. A change to the CI itself or to the dependency lock marks every group true.

A job that a group skips reports "skipped", which satisfies a required check. Do not move the
groups into workflow-level `paths:` filters: a required workflow that does not start reports no
check, and the pull request waits for it forever.
"""

import json
import subprocess
import sys

# A change here can break any job.
EVERYTHING = (
    ".github/",
    "Cargo.lock",
    "Cargo.toml",
    "rust-toolchain.toml",
    "deny.toml",
    "scripts/ci-changes.py",
)

GROUPS = ("rust", "windows", "ui", "openapi", "proto", "docs", "site", "comments", "deps")


def agent_crates():
    """The crates that the agent needs in a normal or build dependency, with the agent itself."""
    metadata = json.loads(
        subprocess.check_output(
            ["cargo", "metadata", "--no-deps", "--format-version", "1", "--offline"], text=True
        )
    )
    deps = {
        package["name"]: [
            dep["name"]
            for dep in package["dependencies"]
            if dep.get("path") and dep.get("kind") != "dev"
        ]
        for package in metadata["packages"]
    }
    seen = set()
    todo = ["restichub-agent"]
    while todo:
        name = todo.pop()
        if name not in seen:
            seen.add(name)
            todo.extend(deps.get(name, []))
    return seen


def starts(path, prefixes):
    return any(path.startswith(prefix) for prefix in prefixes)


def classify(files, agent):
    if any(starts(f, EVERYTHING) for f in files):
        return {group: True for group in GROUPS}
    agent_dirs = tuple(f"crates/{name}/" for name in sorted(agent))
    rust = ("crates/", "proto/", ".cargo/", "packaging/fetch-restic.sh")
    return {
        "rust": any(starts(f, rust) for f in files),
        "windows": any(starts(f, agent_dirs) for f in files),
        "ui": any(starts(f, ("ui/",)) for f in files),
        # The document comes from the control plane; the types are in the UI.
        "openapi": any(starts(f, ("crates/", "ui/", "proto/")) for f in files),
        "proto": any(starts(f, ("proto/",)) for f in files),
        "docs": any(
            starts(f, ("docs/", "design/", "styles/", "scripts/check-step-length.py"))
            or f in ("README.md", "CONTRIBUTING.md", "SECURITY.md", ".vale.ini")
            for f in files
        ),
        "site": any(starts(f, ("docs/",)) for f in files),
        "comments": any(
            starts(f, ("crates/", "scripts/check-comments.sh", "scripts/testdata/")) for f in files
        ),
        "deps": any(starts(f, ("crates/", "proto/", ".cargo/")) for f in files),
    }


def package_args(agent):
    return " ".join(f"-p {name}" for name in sorted(agent))


def main(argv):
    agent = agent_crates()
    if "--all" in argv:
        result = {group: True for group in GROUPS}
    else:
        base = argv[argv.index("--base") + 1]
        files = subprocess.check_output(
            ["git", "diff", "--name-only", base, "HEAD"], text=True
        ).splitlines()
        result = classify(files, agent)
        print(f"changed files: {len(files)}", file=sys.stderr)
    for group in GROUPS:
        print(f"{group}={'true' if result[group] else 'false'}")
    print(f"agent-packages={package_args(agent)}")


if __name__ == "__main__":
    main(sys.argv[1:])
