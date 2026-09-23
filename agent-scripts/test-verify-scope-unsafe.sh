#!/usr/bin/env bash
# Regression test for RES-4861: lexer spellings of the unsafe keyword must
# pass the diff-shape check while actual unsafe Rust constructs remain blocked.
set -euo pipefail

SOURCE_ROOT="$(git rev-parse --show-toplevel)"
TMP="$(mktemp -d)"
trap "rm -rf \"$TMP\"" EXIT
ORIGIN="$TMP/origin.git"
WORK="$TMP/work"

git init --quiet --bare --initial-branch=main "$ORIGIN"
git init --quiet "$WORK"
cd "$WORK"
git config user.name "Guardrail Self-Test"
git config user.email "guardrail-self-test@example.invalid"
git remote add origin "$ORIGIN"
git switch --quiet -c main
mkdir -p agent-scripts resilient/src
cp "$SOURCE_ROOT/agent-scripts/verify-scope.sh" agent-scripts/verify-scope.sh
cat > agent-scripts/check-overlaps.sh <<"STUB"
#!/usr/bin/env bash
exit 0
STUB
chmod +x agent-scripts/check-overlaps.sh
printf "pub fn baseline() {}\n" > resilient/src/lexer.rs
git add agent-scripts resilient/src/lexer.rs
git commit --quiet -m "fixture base"
git push --quiet -u origin main

run_guardrail() {
  bash agent-scripts/verify-scope.sh \
    --base origin/main \
    --head HEAD \
    --skip tests \
    --skip clippy \
    --skip fmt
}

# The source word is valid as a Logos token and in explanatory comments.
git switch --quiet -c safe-keyword main
printf "%s\n" "#[token(\"unsafe\")]" "// unsafe is a source-language keyword" > resilient/src/lexer.rs
git add resilient/src/lexer.rs
git commit --quiet -m "fixture safe keyword spelling"
if run_guardrail > "$TMP/safe.out" 2>&1; then
  echo "case safe keyword text passed"
else
  cat "$TMP/safe.out" >&2
  echo "FAIL: safe keyword text was rejected" >&2
  exit 1
fi

unsafe_snippets=(
  "unsafe {}"
  "unsafe fn added() {}"
  "unsafe impl Send for Marker {}"
  "unsafe trait Marker {}"
  "unsafe extern \"C\" {}"
)
index=0
for snippet in "${unsafe_snippets[@]}"; do
  index=$((index + 1))
  git switch --quiet -c "unsafe-case-$index" main
  printf "%s\n" "$snippet" > resilient/src/unsafe_construct.rs
  git add resilient/src/unsafe_construct.rs
  git commit --quiet -m "fixture unsafe construct $index"
  if run_guardrail > "$TMP/unsafe.out" 2>&1; then
    echo "FAIL: unsafe construct $index was accepted" >&2
    exit 1
  fi
  if ! grep -Fq "introduces new" "$TMP/unsafe.out"; then
    cat "$TMP/unsafe.out" >&2
    echo "FAIL: unsafe construct $index failed for an unrelated reason" >&2
    exit 1
  fi
  echo "case unsafe construct $index rejected"
done
