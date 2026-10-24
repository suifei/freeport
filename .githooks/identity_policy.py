"""Repository-owner identity checks for local Git hooks (Python 3)."""
import re
import subprocess
import sys

EXPECTED = "flynn <7343085+suifei@users.noreply.github.com>"

def git(*args, check=True):
    return subprocess.run(["git", *args], text=True, encoding="utf-8", capture_output=True, check=check)

def reject(reason):
    print("Git operation rejected: " + reason, file=sys.stderr)
    raise SystemExit(1)

def check_current_identity():
    for kind in ("GIT_AUTHOR_IDENT", "GIT_COMMITTER_IDENT"):
        if not git("var", kind).stdout.startswith(EXPECTED + " "):
            reject(kind + " must be the repository owner")

def check_commit_message(message):
    if re.search(r"^\s*(co-authored-by:|generated[- ]by:|generated with|authored[- ]by:)", message, re.I | re.M):
        reject("author/tool attribution trailers are not allowed")
