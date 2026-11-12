#!/usr/bin/env python3
"""Lists the files changed on this branch and the documents they may affect.

Changed files = commits on this branch since it left the base branch, plus staged,
unstaged and untracked files. For each, the rules below say which documents to review.
Development chapters that mention a changed file are listed too, since their excerpts
or explanations may now be out of date.

Usage: docs_impact.py [--repo PATH] [--base main] [--json]
"""

import argparse
import fnmatch
import glob
import json
import os
import re
import subprocess
import sys

# (glob pattern, documents to review, why)
RULES = [
    ("src/api/*", ["docs/user/api.md", "docs/architecture.md"], "endpoints, parameters, responses or error codes"),
    ("src/config.rs", ["docs/user/configuration.md", ".env.example", "docs/user/deploy.md"], "settings and their defaults"),
    ("src/upstream/*", ["docs/architecture.md", "CLAUDE.md"], "upstream behavior (the 'Data source' notes in CLAUDE.md)"),
    ("src/main.rs", ["docs/architecture.md"], "startup and subcommands"),
    ("src/*.rs", ["docs/architecture.md"], "how the service works and the code map"),
    ("src/*/*.rs", ["docs/architecture.md"], "how the service works and the code map"),
    ("migrations/*", ["docs/architecture.md"], "the 'Data' section and table list"),
    ("Cargo.toml", ["CLAUDE.md", "docs/architecture.md"], "stack and dependencies"),
    ("Dockerfile", ["docs/user/deploy.md", "README.md"], "image, sizes, health check"),
    ("docker-compose.yml", ["docs/user/deploy.md", "README.md"], "deployment summary table and steps"),
    (".env.example", ["docs/user/configuration.md", "docs/user/deploy.md"], "settings"),
    ("scripts/*", ["docs/user/deploy.md", "README.md"], "build and release commands"),
    ("tests/*", ["docs/development/README.md"], "test layout and helpers"),
    (".github/workflows/*", ["docs/development/README.md"], "the checks CI runs"),
    ("web/package.json", ["docs/development/README.md", "CLAUDE.md"], "frontend commands, checks and dependencies"),
    ("web/*", ["docs/architecture.md"], "the web app: structure, screens and build"),
]

CODE_PREFIXES = ("src/", "migrations/", "tests/", "scripts/", ".github/", "web/")
CODE_FILES = ("Dockerfile", "docker-compose.yml", ".env.example", "Cargo.toml")


def git(repo, *args):
    result = subprocess.run(["git", "-C", repo, *args], capture_output=True, text=True)
    return result.stdout if result.returncode == 0 else ""


def changed_files(repo, base, scope="worktree"):
    """Files changed since the branch left `base`.

    scope: "branch" = commits only; "index" = commits + staged (what a commit would contain);
    "tracked" = index + unstaged edits of tracked files (`git commit -a`);
    "worktree" = everything, untracked files included.
    """
    merge_base = git(repo, "merge-base", base, "HEAD").strip()
    names = set()
    if merge_base:
        names |= set(git(repo, "diff", "--name-only", merge_base, "HEAD").split())
    if scope != "branch":
        names |= set(git(repo, "diff", "--name-only", "--cached").split())
    if scope in ("tracked", "worktree"):
        names |= set(git(repo, "diff", "--name-only").split())
    if scope == "worktree":
        names |= set(git(repo, "ls-files", "--others", "--exclude-standard").split())
    return sorted(names)


def is_code(path):
    return path.startswith(CODE_PREFIXES) or path in CODE_FILES


def chapters(repo):
    return sorted(glob.glob(os.path.join(repo, "docs", "development", "[0-9][0-9]-*.md")))


def analyze(repo, base, scope="worktree", root=None):
    """`root` is a folder holding the files to read (a snapshot); git always runs in `repo`."""
    root = root or repo
    changed = changed_files(repo, base, scope)
    code = [path for path in changed if is_code(path)]
    docs = [path for path in changed if path.startswith("docs/") or path in ("README.md", "CLAUDE.md", "PLAN.md")]

    review = {}
    for path in code:
        for pattern, targets, why in RULES:
            if fnmatch.fnmatch(path, pattern):
                for target in targets:
                    review.setdefault(target, set()).add(f"{path} ({why})")

    mentioned = {}
    for chapter in chapters(root):
        with open(chapter, encoding="utf-8") as handle:
            text = handle.read()
        hits = [path for path in code if path in text]
        if hits:
            mentioned[os.path.relpath(chapter, root)] = hits

    existing = [os.path.basename(path) for path in chapters(root)]
    numbers = [int(name[:2]) for name in existing if re.match(r"\d\d-", name)]
    branch = git(repo, "rev-parse", "--abbrev-ref", "HEAD").strip()

    return {
        "branch": branch,
        "base": base,
        "code_changed": code,
        "docs_changed": docs,
        "review": {target: sorted(reasons) for target, reasons in sorted(review.items())},
        "chapters_mentioning_changed_files": mentioned,
        "existing_chapters": existing,
        "next_chapter_number": f"{(max(numbers) + 1) if numbers else 1:02d}",
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--repo", default=os.getcwd())
    parser.add_argument("--base", default="main")
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()
    report = analyze(os.path.abspath(args.repo), args.base)

    if args.json:
        print(json.dumps(report, indent=2))
        return 0

    print(f"Branch {report['branch']} vs {report['base']}")
    if not report["code_changed"]:
        print("No code changes: nothing to document.")
        return 0
    print("\nCode changed:")
    for path in report["code_changed"]:
        print(f"  {path}")
    print("\nDocs changed so far:" if report["docs_changed"] else "\nDocs changed so far: none")
    for path in report["docs_changed"]:
        print(f"  {path}")
    print("\nReview these documents:")
    for target, reasons in report["review"].items():
        print(f"  {target}")
        for reason in reasons:
            print(f"      because of {reason}")
    if report["chapters_mentioning_changed_files"]:
        print("\nDevelopment chapters that mention changed files (check excerpts and 'Later changes'):")
        for chapter, hits in report["chapters_mentioning_changed_files"].items():
            print(f"  {chapter}: {', '.join(hits)}")
    print(f"\nDevelopment chapter for this step: update the one that covers it, or create "
          f"docs/development/{report['next_chapter_number']}-<name>.md "
          f"(existing: {', '.join(report['existing_chapters']) or 'none'}).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
