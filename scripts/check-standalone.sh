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
set -euo pipefail
cd "$(dirname "$0")/.."

EXCLUDES=(
  ':(exclude)*/__fixtures__/*'
  ':(exclude)*/__oracle__/*'
  ':(exclude)Cargo.lock'
  ':(exclude)package-lock.json'
)

fail=0

# 1. Path deps escaping the repo. A sibling hop (`../<crate>`) is fine; two or
#    more hops leaves crates/ and can only resolve inside xyd.
if git grep -n -E 'path *= *"\.\./\.\.' -- '*/Cargo.toml' "${EXCLUDES[@]}"; then
  echo "ERROR: a Cargo.toml path dep escapes this repo (see above)." >&2
  fail=1
fi

# 2. Filesystem reaches into xyd's layout, in code (not comments or goldens).
if git grep -n -E '"(\.\./)*packages/xyd-' -- '*.rs' "${EXCLUDES[@]}"; then
  echo "ERROR: Rust code resolves a path under xyd's packages/ (see above)." >&2
  fail=1
fi

# 3. npm imports of xyd's workspace packages.
if git grep -n -E "from ['\"]@xyd-js/" -- '*.ts' '*.mjs' '*.js' "${EXCLUDES[@]}"; then
  echo "ERROR: an @xyd-js/* import would not resolve standalone (see above)." >&2
  fail=1
fi

# Non-vacuity: if the pathspec ever stops matching anything, the greps above
# pass for the wrong reason. Assert the corpus they scan is actually there.
manifests=$(git ls-files '*/Cargo.toml' -- "${EXCLUDES[@]}" | wc -l | tr -d ' ')
if [ "$manifests" -lt 20 ]; then
  echo "ERROR: only $manifests crate manifests scanned (expected >= 20) — the" >&2
  echo "       pathspec is broken, so a clean result proves nothing." >&2
  fail=1
fi

[ "$fail" -eq 0 ] && echo "check-standalone: OK ($manifests manifests scanned)"
exit "$fail"
