#!/usr/bin/env python3
"""Select the fitness checks (fitness/register.toml) a change triggers.

    select-checks.py <base>            checks triggered by `git diff --name-only <base>`
    select-checks.py --files A B ...   checks triggered by these files
    select-checks.py --all             every check that runs on a hosted runner (weekly, release, dispatch)
    add --jobs                         print the distinct CI jobs of the selection instead of the check ids

Prints one id (or job) per line, nothing when no check is triggered. A changed file under src/ triggers the checks
that list its module's class; any file matching one of a check's paths triggers it too. Only `where = "ci"` checks
are ever selected: a reference-machine check is run by hand at release.
"""
import fnmatch
import re
import subprocess
import sys
import tomllib
from pathlib import Path

REGISTER = Path(__file__).resolve().parent.parent / "fitness" / "register.toml"


def module_of(path):
    """The src/ module a file belongs to (`src/config/schema.rs` is `config`), or None outside src/."""
    match = re.fullmatch(r"src/([^/]+?)(?:\.rs|/.*)", path)
    return match.group(1) if match else None


def select(register, files, everything=False):
    modules = register["modules"]
    chosen = []
    for check in register["checks"]:
        if check["where"] != "ci":
            continue
        triggers = check["triggers"]
        if everything:
            hit = True
        else:
            classes = {modules.get(module_of(f)) for f in files} - {None}
            hit = bool(classes & set(triggers["classes"])) or any(
                fnmatch.fnmatchcase(f, glob) for f in files for glob in triggers["paths"]
            )
        if hit:
            chosen.append(check)
    return chosen


def main(argv):
    jobs = "--jobs" in argv
    args = [a for a in argv if a != "--jobs"]
    register = tomllib.loads(REGISTER.read_text())
    if args == ["--all"]:
        chosen = select(register, [], everything=True)
    elif args[:1] == ["--files"]:
        chosen = select(register, args[1:])
    elif len(args) == 1 and not args[0].startswith("-"):
        out = subprocess.run(
            ["git", "diff", "--name-only", args[0]], check=True, capture_output=True, text=True
        ).stdout
        chosen = select(register, out.split())
    else:
        print(__doc__, file=sys.stderr)
        return 2
    names = [c["job"] for c in chosen] if jobs else [c["id"] for c in chosen]
    for name in dict.fromkeys(names):
        print(name)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
