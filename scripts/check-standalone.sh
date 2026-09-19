#!/usr/bin/env bash
# Guard: this repo must not reach back into xyd.
#
# The crates keep their `xyd_*` names on purpose (renaming them would have
# destroyed the tree-SHA proof the extraction rests on), so the thing to check
# is not the NAME but the REACH: a path dep, a filesystem path, or an import
# that only resolves inside the xyd monorepo.
#
# Excludes are generated data and prose. Goldens are oracles — rewriting them
# to match the code would defeat the point of having them; doc comments
# legitimately reference where a file used to live in xyd's history.
#
# Every guard reports the size of the corpus it scanned. A guard that matches
# nothing because its pathspec is broken, or because the corpus is empty, is
# indistinguishable from a guard that passed — and a single shared counter
# cannot vouch for three greps over three different file sets.
set -euo pipefail
cd "$(dirname "$0")/.."

EXCLUDES=(
  ':(exclude)*/__fixtures__/*'
  ':(exclude)*/__oracle__/*'
  ':(exclude)Cargo.lock'
  ':(exclude)package-lock.json'
)

fail=0

# guard <label> <min-corpus> <regex> <pathspec>...
#
# min-corpus is the floor below which a clean result proves nothing. Pass 0 for
# a corpus that may legitimately be empty — it is then REPORTED as inapplicable
# rather than counted as a pass.
guard() {
  local label="$1" min="$2" regex="$3"; shift 3
  local paths=("$@")
  local n status

  n=$(git ls-files -- "${paths[@]}" "${EXCLUDES[@]}" | wc -l | tr -d ' ')

  if [ "$n" -lt "$min" ]; then
    echo "ERROR: [$label] scanned only $n file(s), expected >= $min — the" >&2
    echo "       pathspec is broken, so a clean result proves nothing." >&2
    fail=1
    return
  fi

  if [ "$n" -eq 0 ]; then
    echo "  [$label] no files match this corpus — check INAPPLICABLE (not a pass)"
    return
  fi

  # git grep: 0 = matched, 1 = no match, >1 = real error. `if git grep ...`
  # would collapse 1 and 2 into "clean", so a broken invocation would read as a
  # pass. Discriminate explicitly.
  set +e
  git grep -n -E "$regex" -- "${paths[@]}" "${EXCLUDES[@]}"
  status=$?
  set -e

  case "$status" in
    0) echo "ERROR: [$label] matched (see above) across $n file(s)." >&2; fail=1 ;;
    1) echo "  [$label] clean ($n file(s) scanned)" ;;
    *) echo "ERROR: [$label] git grep failed with status $status." >&2; fail=1 ;;
  esac
}

# 1. Path deps escaping the repo. Crates sit at two depths — `crates/<name>` and
#    the root-level `cli` — so "one hop good, two hops bad" is not the rule: the
#    CLI legitimately reaches its dependencies as `../crates/<name>`. What can
#    never resolve inside this repo is a `../..` prefix, from either depth.
#    (`crates/opensdk_core/tests/layers.rs` does the rigorous version of this
#    check, resolving every declared path and asserting it lands under the root.)
guard "path-deps" 20 'path *= *"\.\./\.\.' '*/Cargo.toml'

# 2. Filesystem reaches into xyd's layout, in code (not comments or goldens).
guard "rust-paths" 50 '"(\.\./)*packages/xyd-' '*.rs'

# 3. npm imports of xyd's workspace packages. The JS corpus is legitimately
#    empty today (this repo is pure Rust plus a lockfile), so the floor is 0 and
#    an empty scan is reported as inapplicable rather than green.
guard "npm-imports" 0 "from ['\"]@xyd-js/" '*.ts' '*.mjs' '*.js'

[ "$fail" -eq 0 ] && echo "check-standalone: OK"
exit "$fail"
