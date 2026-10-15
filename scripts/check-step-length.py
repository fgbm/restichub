#!/usr/bin/env python3
"""Check that each sentence in a numbered procedure step has 20 words or fewer (AC-DOC-01).

Vale counts a whole list item, so the Vale rule is only a warning. This script
counts each sentence of a numbered list item, with its continuation lines.
"""
import re
import sys
from pathlib import Path

LIMIT = 20
STEP = re.compile(r"^(\s*)\d+\.\s+(.*)$")
WORD = re.compile(r"[\w\-/.]*[^\W_][\w\-/.]*")


def clean(text):
    text = re.sub(r"`[^`]*`", "code", text)
    text = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", text)
    return text


def sentences(text):
    return [s for s in re.split(r"(?<=[.!?:])\s+(?=[A-ZА-ЯЁ])", clean(text)) if s.strip()]


def check(path):
    failures = 0
    lines = path.read_text(encoding="utf-8").splitlines()
    in_fence = False
    i = 0
    while i < len(lines):
        line = lines[i]
        if line.lstrip().startswith("```"):
            in_fence = not in_fence
        m = None if in_fence else STEP.match(line)
        if m:
            start = i + 1
            indent = len(m.group(1))
            text = m.group(2)
            j = i + 1
            while j < len(lines) and lines[j].strip() and not STEP.match(lines[j]) \
                    and not lines[j].lstrip().startswith(("```", "- ", "* ")) \
                    and len(lines[j]) - len(lines[j].lstrip()) > indent:
                text += " " + lines[j].strip()
                j += 1
            for s in sentences(text):
                n = len(WORD.findall(s))
                if n > LIMIT:
                    print(f"{path}:{start}: step sentence has {n} words (limit {LIMIT}): {s[:70]}")
                    failures += 1
            i = j
            continue
        i += 1
    return failures


total = 0
for root in sys.argv[1:] or ["docs/src"]:
    for p in sorted(Path(root).rglob("*.md*")):
        total += check(p)
sys.exit(1 if total else 0)
