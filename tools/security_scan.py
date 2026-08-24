#!/usr/bin/env python3
"""Security pre-push scan for Z Desktop.

Scans tracked-candidate files for secret-looking patterns.
Exit code 0 = clean, 1 = findings (paths + line numbers only, never values).
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# Directories that must never be scanned or published
EXCLUDE_DIRS = {
    "target", ".git", "node_modules", "references", "deepseek-harness",
    "hermes-agent", "zero final", "data",
}
EXCLUDE_FILES = {".env", "credentials.json"}
EXCLUDE_EXT = {".pdb", ".exe", ".dll", ".lock"}

DQ = "\\x22"  # double quote, kept as hex so this file survives any pipeline
SQ = "\\x27"  # single quote

PATTERNS = [
    ("openai-style-key", re.compile(r"sk-[A-Za-z0-9_\-]{20,}")),
    ("anthropic-key", re.compile(r"sk-ant-[A-Za-z0-9_\-]{20,}")),
    ("xai-key", re.compile(r"xai-[A-Za-z0-9]{20,}")),
    ("github-token", re.compile(r"gh[pousr]_[A-Za-z0-9]{30,}")),
    ("aws-access-key", re.compile(r"AKIA[0-9A-Z]{16}")),
    ("google-api-key", re.compile(r"AIza[0-9A-Za-z_\-]{35}")),
    ("private-key-block", re.compile(r"-----BEGIN [A-Z ]*PRIVATE KEY-----")),
    ("bearer-assignment", re.compile(
        "(?i)(api[_-]?key|secret|password)\\s*[=:]\\s*[" + DQ + SQ + "][^"
        + DQ + SQ + "]{12,}[" + DQ + SQ + "]")),
    ("generic-token-url", re.compile(r"https://[^\s]*:[^\s@]{8,}@")),
]


def iter_files():
    for path in ROOT.rglob("*"):
        if not path.is_file():
            continue
        rel = path.relative_to(ROOT)
        parts = set(rel.parts)
        if parts & EXCLUDE_DIRS:
            continue
        if path.name in EXCLUDE_FILES or path.suffix.lower() in EXCLUDE_EXT:
            continue
        yield path


# Allowlisted synthetic fixtures: substrings that appear ONLY inside
# verified test code (e.g., redaction unit tests) and are known to be
# fake. A match line containing one of these is reported as allowlisted,
# not as a finding. Keep this list minimal and reviewed.
ALLOWLIST_SUBSTRINGS = [
    "sk-proj-abcdefghij",          # redact.rs test fixture (fake)
    "xai-abcdefghijklmnopqrst",    # redact.rs test fixture (fake)
    "ghp_0123456789abcdef",        # redact.rs test fixture (fake)
    "AKIAIOSFODNN7EXAMPLE",        # AWS documentation example key
    "AIzaSyA1234567890",           # redact.rs test fixture (fake)
    "sk-pro...6789",               # redact.rs test placeholder (fake)
    "ghp_01...wxyz",               # redact.rs test placeholder (fake)
    "AKIAIO...MPLE",               # redact.rs test placeholder (fake)
    "sk-SU",                       # runtime.rs journal test placeholder prefix
                                   # (21-char fake; the test asserts it never persists)
    "sk-abcdefghijklmnopqrstuvwx",  # reducer/redact test fixtures (full alphabet fake)
    "sk-fake1234567890abcdefghijklmn",  # redact.rs strict-gate test fixture (fake)
]


def is_allowlisted(line: str) -> bool:
    return any(marker in line for marker in ALLOWLIST_SUBSTRINGS)


def main():
    findings = 0
    allowlisted = 0
    for path in iter_files():
        try:
            text = path.read_text(encoding="utf-8", errors="ignore")
        except OSError:
            continue
        for lineno, line in enumerate(text.splitlines(), 1):
            for name, pattern in PATTERNS:
                if pattern.search(line):
                    if is_allowlisted(line):
                        allowlisted += 1
                        continue
                    print(f"{name}: {path.relative_to(ROOT)}:{lineno}")
                    findings += 1
    if allowlisted:
        print(f"({allowlisted} allowlisted synthetic test fixture(s) skipped)")
    if findings:
        print()
        print(f"{findings} potential secret(s) found - review before push.")
        return 1
    print("clean: no secret patterns found in publishable files")
    return 0


if __name__ == "__main__":
    sys.exit(main())