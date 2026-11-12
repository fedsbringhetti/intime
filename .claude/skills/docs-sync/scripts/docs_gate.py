#!/usr/bin/env python3
"""Claude Code PreToolUse hook: stops `git commit` and `gh pr create` when the docs are behind.

Reads the hook payload (JSON) from stdin. For any other command it does nothing. For a
commit or a PR it blocks (exit code 2, reason on stderr, which Claude reads) when:
- code changed on the branch and no documentation file changed, or
- check_docs.py finds broken links or code excerpts that no longer match the code.

What is checked depends on the command, so unrelated edits in the working folder do not matter:
- `git commit`: the index (a temporary copy of it) and the branch plus staged changes against
  main. With `-a` the tracked edits count too. If the same command stages first (`git add ...
  && git commit`) the index is not final yet, so the working folder is checked instead.
- `gh pr create`: the committed branch (HEAD), against main.

Only real commands are matched: heredoc bodies and quoted strings are ignored.

Add "[skip docs]" to the commit message (or the PR command) for a change with no
documentation impact, such as a pure refactor or a dependency bump.
"""

import json
import os
import re
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

from docs_impact import analyze  # noqa: E402

SKIP = "[skip docs]"
COMMIT = re.compile(r"^git(?:\s+(?:-C\s+\S+|-c\s+\S+|--[\w-]+(?:=\S+)?))*\s+commit(?:\s|$)(.*)$")
PR = re.compile(r"^gh\s+pr\s+create(?:\s|$)")
STAGING = re.compile(r"^git(?:\s+(?:-C\s+\S+|-c\s+\S+|--[\w-]+(?:=\S+)?))*\s+(?:add|rm|mv|reset|restore|checkout|switch|stash)(?:\s|$)")
ALL_FLAG = re.compile(r"(?:^|\s)(?:--all|-[a-zA-Z]*a[a-zA-Z]*)(?:\s|$)")
HEREDOC = re.compile(r"<<-?\s*(['\"]?)([A-Za-z_]\w*)\1")


def strip_heredocs(command):
    """Drops heredoc bodies; the line that opens one is kept."""
    out, pending = [], []
    for line in command.split("\n"):
        if pending:
            if line.strip() == pending[0]:
                pending.pop(0)
            continue
        out.append(line)
        pending = [m.group(2) for m in HEREDOC.finditer(line)]
    return "\n".join(out)


def segments(command):
    """Splits a shell command into its simple commands; quoted text becomes `x`."""
    text, parts, current, quote, i = strip_heredocs(command), [], [], None, 0
    while i < len(text):
        ch = text[i]
        if quote:
            if ch == "\\" and quote == '"':
                i += 1
            elif ch == quote:
                quote = None
        elif ch == "\\":
            i += 1
        elif ch in "'\"":
            quote = ch
            current.append("x")
        elif ch in ";&|\n()`{}":
            parts.append("".join(current))
            current = []
        else:
            current.append(ch)
        i += 1
    parts.append("".join(current))
    cleaned = []
    for part in parts:
        words = part.split()
        while words and re.match(r"^\w+=\S*$", words[0]):
            words.pop(0)
        if words:
            cleaned.append(" ".join(words))
    return cleaned


def classify(command):
    """Returns (kind, scope) for the first real commit or PR command, else (None, None)."""
    staged_first = False
    for segment in segments(command):
        match = COMMIT.match(segment)
        if match:
            if staged_first:
                return "commit", "worktree"
            return "commit", "tracked" if ALL_FLAG.search(match.group(1)) else "index"
        if PR.match(segment):
            return "pr", "branch"
        if STAGING.match(segment):
            staged_first = True
    return None, None


def git(repo, *args):
    return subprocess.run(["git", "-C", repo, *args], capture_output=True, text=True)


def snapshot(repo, kind, scope, target):
    """Writes the files the commit or the PR will contain into `target`."""
    if kind == "pr":
        archive = subprocess.run(["git", "-C", repo, "archive", "HEAD"], capture_output=True)
        subprocess.run(["tar", "-x", "-C", target], input=archive.stdout, check=True)
        return
    if scope == "worktree":
        listed = git(repo, "ls-files", "--cached", "--others", "--exclude-standard", "-z").stdout
        for name in filter(None, listed.split("\0")):
            copy(repo, target, name)
        return
    git(repo, "checkout-index", "-a", "--prefix", target + os.sep)
    if scope == "tracked":
        for name in filter(None, git(repo, "diff", "--name-only", "-z").stdout.split("\0")):
            copy(repo, target, name)


def copy(repo, target, name):
    source, dest = os.path.join(repo, name), os.path.join(target, name)
    if os.path.isfile(source):
        os.makedirs(os.path.dirname(dest), exist_ok=True)
        shutil.copy2(source, dest)
    elif os.path.lexists(dest):
        os.remove(dest)


def main():
    try:
        payload = json.load(sys.stdin)
    except json.JSONDecodeError:
        return 0
    command = (payload.get("tool_input") or {}).get("command") or ""
    kind, scope = classify(command)
    if kind is None or SKIP in command:
        return 0

    repo = os.environ.get("CLAUDE_PROJECT_DIR") or payload.get("cwd") or os.getcwd()
    problems = []
    with tempfile.TemporaryDirectory(prefix="docs-gate-") as target:
        snapshot(repo, kind, scope, target)
        report = analyze(repo, "main", scope, root=target)

        if report["code_changed"] and not report["docs_changed"]:
            lines = [
                "Code changed on this branch but no documentation was updated.",
                "Run the docs-sync skill (or follow it by hand), then retry. Documents to review:",
            ]
            lines += [f"  - {t}" for t in report["review"]]
            lines.append(f"  - docs/development/: chapter for this step (next number {report['next_chapter_number']})")
            what = "commit message" if kind == "commit" else "command"
            lines.append(f'If this change truly has no documentation impact, add "{SKIP}" to the {what}.')
            problems.append("\n".join(lines))

        check = subprocess.run(
            [sys.executable, "-I", os.path.join(HERE, "check_docs.py"), "--repo", target],
            capture_output=True,
            text=True,
        )
        if check.returncode != 0:
            problems.append(check.stdout.strip().replace(target, "."))

    if problems:
        print("\n\n".join(problems), file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
