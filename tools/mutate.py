#!/usr/bin/env python3
"""Mutation sweep driver: applies one small wrong change at a time and says which test caught it.

    ROOT=<copy of the tree> DATABASE_URL=... tools/mutate.py <mutants.py>

<mutants.py> is Python that sets `mutants`, a list of `(name, path, old, new)`: in the file
`path` (relative to ROOT) the text `old`, which must occur exactly once, in code and above the
file's test module, is replaced by `new`. Each mutant prints one line:

    <name> -> compile-kill | killed by <tests> | SURVIVED

A survivor is a wrong line no test notices: write the test, or show the mutant is equivalent
and test the real defect it stands for (AGENTS.md, "VERIFICATION AND SEARCH RULES").

ROOT must be a copy of the tree with `legacy/` and `docs/parity/` in it, never the tree
you work in: the sweep rewrites source files in place, and although it restores each one by
checksum, a sweep that is killed leaves a mutated file behind.

Environment:
    ROOT            the copy to mutate and test (required)
    DATABASE_URL    the test server; the scenarios do not run without it (required)
    CHECK=1         only check that every mutant applies, then stop
    ONLY=a,b        run only the mutants with these names
    SHARD=k/n       run the mutants whose index is k modulo n, so n copies can share a sweep
    BASELINE=0      skip the unmutated run that proves the suites are green first
    PARITY_THREADS  cap the scenario suite's threads on a slow machine
    FLAKY_DIR       keep the output of every scenario run that failed only under load

A scenario failure counts as a kill only when the failing scenarios fail again run alone on one
thread, so a scenario that timed out under load does not read as a kill. This driver is the one
ledgers 229b to 229k used (`mutate_v2.py`), with the paths of that machine taken out.
"""
import hashlib
import os
import re
import subprocess
import sys

if len(sys.argv) != 2 or not os.environ.get("ROOT") or not os.environ.get("DATABASE_URL"):
    sys.exit(__doc__)
with open(sys.argv[1], encoding="utf-8") as source:
    scope = {}
    exec(source.read(), scope)
mutants = scope["mutants"]
os.chdir(os.environ["ROOT"])
assert os.path.isdir("docs/parity"), "ROOT has no docs/parity, so every scenario run fails"
assert os.path.isdir("legacy"), "ROOT has no legacy/, so no server starts"

only = os.environ.get("ONLY")
if only:
    mutants = [m for m in mutants if m[0] in only.split(",")]
shard = os.environ.get("SHARD")
if shard:
    k, n = map(int, shard.split("/"))
    mutants = [m for i, m in enumerate(mutants) if i % n == k]

BASE = ["--locked", "--offline", "--no-fail-fast"]
WORLD = ["cargo", "test", "-p", "world", "--lib"] + BASE
COMMON = ["cargo", "test", "-p", "common", "--lib"] + BASE
GAMEDATA = ["cargo", "test", "-p", "gamedata", "--lib"] + BASE
QUEST = ["cargo", "test", "-p", "quest"] + BASE
DB = ["cargo", "test", "-p", "db"] + BASE
PROTOCOL = ["cargo", "test", "-p", "protocol"] + BASE
NET = ["cargo", "test", "-p", "net"] + BASE
PARITY = ["cargo", "test", "-p", "parity"] + BASE
BY_CRATE = (("crates/world/", WORLD), ("crates/common/", COMMON), ("crates/gamedata/", GAMEDATA),
            ("crates/quest/", QUEST), ("crates/db/", DB), ("crates/protocol/", PROTOCOL),
            ("crates/net/", NET), ("crates/parity/", PARITY))
PRODOMO_LIB = ["cargo", "test", "-p", "prodomo", "--lib"] + BASE
# The binary's own unit tests (`main.rs`'s test module), which `--lib` does not build.
PRODOMO_BIN = ["cargo", "test", "-p", "prodomo", "--bin", "prodomo"] + BASE
SCENARIOS_ALONE = ["cargo", "test", "-p", "prodomo", "--test", "parity"] + BASE
threads = os.environ.get("PARITY_THREADS")
SCENARIOS = SCENARIOS_ALONE + (["--", "--test-threads=" + threads] if threads else [])
# A scenario run that passed fewer than a hundred tests did not run the suite.
SCENARIOS_RAN = re.compile(r"test result: ok\. \d{3,} passed")


def run(command):
    done = subprocess.run(command, capture_output=True, text=True)
    return done.returncode, done.stdout + done.stderr


def failures(out):
    return [l.split()[1] for l in out.splitlines() if l.startswith("test ") and l.endswith("FAILED")]


def tests_start(text):
    """The file's test module: a `#[cfg(test)]` at column 0. An indented one marks a helper,
    and one over `mod name;` declares a module kept in a file of its own."""
    found = re.search(r"(?m)^#\[cfg\(test\)\]\n(?!mod \w+;)", text)
    return found.start() if found else -1


def suites(path):
    """The cheapest suite that could notice a change in `path` first, the scenarios last."""
    runs = [suite for crate, suite in BY_CRATE if path.startswith(crate)]
    if path == "crates/prodomo/src/main.rs":
        runs.append(PRODOMO_BIN)
    return runs + [PRODOMO_LIB, SCENARIOS]


def read(path):
    with open(path, encoding="utf-8") as source:
        return source.read()


def write(path, text):
    with open(path, "w", encoding="utf-8") as target:
        target.write(text)


for name, path, old, new in mutants:
    text = read(path)
    assert text.count(old) == 1, (name, "occurs", text.count(old), "times")
    # A change that lands in a comment proves nothing.
    assert not all(l.strip().startswith("//") for l in old.splitlines() if l.strip()), name
    assert text.replace(old, new) != text, name
    tests_at = tests_start(text)
    assert tests_at < 0 or text.find(old) < tests_at, (name, "inside the tests")
if os.environ.get("CHECK"):
    print("check done:", len(mutants), "mutants apply", flush=True)
    sys.exit(0)

if os.environ.get("BASELINE", "1") != "0":
    for suite in [suite for _, suite in BY_CRATE] + [PRODOMO_LIB, PRODOMO_BIN, SCENARIOS]:
        code, out = run(suite)
        assert code == 0, ("baseline", suite, failures(out)[:5])
        if suite is SCENARIOS:
            assert SCENARIOS_RAN.search(out), "baseline: the scenarios did not run"
    print("baseline ok", flush=True)

for name, path, old, new in mutants:
    text = read(path)
    digest = hashlib.sha256(text.encode()).hexdigest()
    write(path, text.replace(old, new))
    verdict, note = None, ""
    try:
        for suite in suites(path):
            code, out = run(suite)
            if "error[" in out or "could not compile" in out:
                verdict = "compile-kill"
                break
            if code != 0:
                failed = failures(out)
                if suite is SCENARIOS and failed:
                    again, alone = run(SCENARIOS_ALONE + ["--", "--exact", "--test-threads=1"] + failed)
                    if os.environ.get("FLAKY_DIR"):
                        write(os.path.join(os.environ["FLAKY_DIR"], name + ".out"), out)
                    if again == 0:
                        note = " (failed only under load: " + ",".join(failed) + ")"
                        continue
                    verdict = "killed by " + ",".join(failures(alone) or failed) + " (confirmed alone)"
                    break
                verdict = "killed by " + ",".join(failed)
                break
            if suite is SCENARIOS:
                assert SCENARIOS_RAN.search(out), (name, "the scenarios did not run")
    finally:
        write(path, text)
        assert hashlib.sha256(read(path).encode()).hexdigest() == digest
    print(name, "->", (verdict or "SURVIVED") + note, flush=True)
print("sweep done", flush=True)
