#!/usr/bin/env bash
# Runs every gate of AGENTS.md ("BUILD AND TEST") and writes the receipt a ledger section needs.
#
#   tools/gates.sh <output-dir>
#
# <output-dir>/summary.txt gets one line per gate, the name of every test that failed, and what
# the tests left behind: files in their temporary folder, servers still running, scratch
# databases on the test server and core files. Anything left fails the run, like a failed gate.
# Each gate's whole output is in <output-dir>/<gate>.log.
#
# The tests run without DATABASE_URL and, when DATABASE_URL is set, a second time with it; the
# scripted-client scenarios run only in that second run. Run it in the background (the two test
# runs take about ten minutes on 16 cores), and never beside another test run on the same
# database server: the scratch databases of that run would be counted as left behind.
set -u

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
out=${1:?usage: tools/gates.sh <output-dir>}
mkdir -p "$out"
out=$(cd "$out" && pwd)
cd "$root" || exit 2

# A rustup proxy in ~/.cargo/bin shadows the distribution's pinned toolchain when it comes first
# in PATH; one gate run was built with the wrong compiler that way (ledger 229j).
if [ -x /usr/bin/cargo ]; then
    PATH=/usr/bin:$PATH
fi
export PATH

summary=$out/summary.txt
: > "$summary"
failed=0

say() {
    printf '%s\n' "$*" | tee -a "$summary"
}

# gate <name> <command...>: runs the command into <name>.log and records its exit code and time.
gate() {
    local name=$1 start rc
    shift
    start=$(date +%s)
    "$@" > "$out/$name.log" 2>&1
    rc=$?
    say "$name: rc=$rc secs=$(($(date +%s) - start))"
    if [ "$rc" -ne 0 ]; then
        failed=1
    fi
}

# tests <name>: after gate <name>, the totals over every target and each failed test by name.
tests() {
    local log=$out/$1.log
    say "$1: $(command grep -a '^test result' "$log" |
        awk '{p += $4; f += $6; i += $8} END {print "passed=" p+0 " failed=" f+0 " ignored=" i+0 " targets=" NR}')"
    command grep -a -E '^test .* FAILED$' "$log" | sort -u | while read -r _ test _; do
        say "$1: FAILED $test"
    done
}

say "toolchain: $(cargo --version 2>&1); $(rustc --version 2>&1)"

# The tests get a temporary folder of this run's own, so that whatever is in it afterwards was
# left by this run and by nothing else. A folder an earlier run left here is that run's leak,
# already reported, and is cleared.
tmp=$out/tmp
rm -rf "$tmp"
mkdir -p "$tmp"

gate fmt cargo fmt --all -- --check
gate build cargo build --workspace --locked --offline
gate clippy cargo clippy --workspace --all-targets --locked --offline -- -D warnings

gate test-nodb env -u DATABASE_URL TMPDIR="$tmp" \
    cargo test --workspace --all-targets --locked --offline --no-fail-fast
tests test-nodb

if [ -n "${DATABASE_URL:-}" ]; then
    gate test-db env TMPDIR="$tmp" \
        cargo test --workspace --all-targets --locked --offline --no-fail-fast
    tests test-db
else
    say "test-db: not run (DATABASE_URL is not set), so no scenario ran"
fi

gate doctest env TMPDIR="$tmp" cargo test --workspace --doc --locked --offline
gate doc env RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked --offline

# left <name> <count> [<what>...]: records what the tests left behind; any of it fails the run.
left() {
    local name=$1 count=$2
    shift 2
    say "$name: $count"
    if [ "$count" -ne 0 ]; then
        failed=1
        printf '%s\n' "$@" | head -n 5 | while read -r what; do
            say "$name: LEFT $what"
        done
    fi
}

# A test removes what it wrote and stops what it started, also when it fails (AGENTS.md,
# "TEST HYGIENE").
mapfile -t files < <(find "$tmp" -mindepth 1 -maxdepth 1 | sort)
left temp-files "${#files[@]}" "${files[@]}"
mapfile -t servers < <(pgrep -a -f -- "--config $tmp/")
left servers "${#servers[@]}" "${servers[@]}"

# The test server of AGENTS.md ("AGENT RUNTIME ENVIRONMENT"); `psql` is only in its container.
if listed=$(podman exec prodomo-pg18 psql -U prodomo -d prodomo -Atc \
    "SELECT datname FROM pg_database WHERE datname ~ '^prodomo_(test|proc|async)_' ORDER BY 1" \
    2> /dev/null); then
    dbs=()
    if [ -n "$listed" ]; then
        mapfile -t dbs <<< "$listed"
    fi
    left scratch-dbs "${#dbs[@]}" "${dbs[@]}"
else
    say "scratch-dbs: not checked (no prodomo-pg18 container answered)"
fi

mapfile -t cores < <(find . \( -path ./target -o -path ./.git -o -path ./.overlord -o -path ./.scratch/tmp \) \
    -prune -o -name '*.core' -print)
left cores "${#cores[@]}" "${cores[@]}"

# `#[allow]` is banned (AGENTS.md, "BUILD AND TEST"); the ones checked in are debt
# (docs/STATUS.md, "Code-quality backlog"), and this count may not rise.
say "allows: $(command grep -a -r -E '#!?\[allow\(' --include='*.rs' \
    protocol common net db gamedata world quest prodomo parity | wc -l)"

if [ "$failed" -ne 0 ]; then
    say "result: FAILED"
    exit 1
fi
say "result: ok"
