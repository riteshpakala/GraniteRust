#!/usr/bin/env bash
# Installs a Rust agent repository and checks what its runner says against the standard, in an Apple `container`
# (Linux, arm64) or, with --native, on this Mac.
#
#   containers/check.sh <agent folder | git URL[@revision]> [--native] [--granite <GraniteRust clone>]
#                       [--out <dir>] [--cache <dir>] [--keep]
#   (GRANITE_CHECK_MEMORY, 8g, and GRANITE_CHECK_CPUS, 8, size each container)
#
# 1. Install it with GraniteRust's installer: cargo metadata → cargo build --release → --describe → installation.json,
#    the record validated against granite-agent-installation.schema.json.
# 2. Describe it: every entry valid against granite-agent-entry.schema.json.
# 3. Every call the agent keeps as a fixture, in its Schema/examples/:
#    *.perform.json  {"perform": {…}, "expect": {"exit": 0}}: answered; the answer valid against Granite's
#                    granite-agent-answer.schema.json and Craft's craft-agent-output.schema.json; every path written;
#                    `made` of kinds it declares; every `expected` made (precision 1).
#    *.refusal.json  {"perform": {…}, "expect": {"exit": 3, "kind": "call"}}: refused with that kind.
#    The examples folder is the call's folder, so a fixture names its files relative to it.
#
# --granite points GraniteRust's GitHub URL at a clone (git's own URL rewriting, for this check only), for an agent
# whose Cargo.lock pins GraniteRust commits not yet pushed. --out keeps each answer, the shelf and the call's folder.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/.." && pwd)
agent="" mode=container granite="" out="" keep=0 cache="${XDG_CACHE_HOME:-$HOME/.cache}/granite-rust"
while [ $# -gt 0 ]; do
  case $1 in
    --native) mode=native ;;
    --granite) granite=$(cd "$2" && pwd); shift ;;
    --out) out=$2; shift ;;
    --cache) cache=$2; shift ;;
    --keep) keep=1 ;;
    -*) echo "unknown option $1" >&2; exit 2 ;;
    *) agent=$1 ;;
  esac
  shift
done
[ -n "$agent" ] || { sed -n '2,20p' "$0"; exit 2; }

scratch=$(mktemp -d "${TMPDIR:-/tmp}/granite-check.XXXXXX")
[ $keep = 1 ] || [ -n "$out" ] || trap 'rm -rf "$scratch"' EXIT
mkdir -p "$scratch/shelf" "$scratch/work" "$cache/registry" "$cache/git"
say() { printf '\n== %s\n' "$*"; }
failed() { echo "FAILED: $*" >&2; exit 1; }

# The agent, as an installer would fetch it: a clone of its commit, never the working folder itself.
if [ -d "$agent" ]; then
  agent_dir=$(cd "$agent" && pwd)
  if git -C "$agent_dir" rev-parse --git-dir >/dev/null 2>&1 && git -C "$agent_dir" rev-parse HEAD >/dev/null 2>&1; then
    [ -z "$(git -C "$agent_dir" status --porcelain)" ] || echo "note: $agent_dir has uncommitted changes; the check installs its last commit" >&2
    git clone --quiet "$agent_dir" "$scratch/agent"
  else
    rsync -a --exclude target --exclude .git "$agent_dir/" "$scratch/agent/"
  fi
  examples="$agent_dir/Schema/examples"
  source_in_container=/scratch/agent
  source_native="$scratch/agent"
else
  url=${agent%@*}
  git clone --quiet "$url" "$scratch/agent-read" 2>/dev/null || failed "could not clone $url"
  examples="$scratch/agent-read/Schema/examples"
  source_in_container=$agent
  source_native=$agent
fi
[ -d "$examples" ] && cp -R "$examples/." "$scratch/work/"

rewrite=()
if [ -n "$granite" ]; then
  rewrite=(GIT_CONFIG_COUNT=2
           "GIT_CONFIG_KEY_0=url.file://GRANITE.insteadOf" "GIT_CONFIG_VALUE_0=https://github.com/riteshpakala/GraniteRust.git"
           "GIT_CONFIG_KEY_1=safe.directory" "GIT_CONFIG_VALUE_1=*")
fi

if [ $mode = container ]; then
  command -v container >/dev/null || failed "Apple's container is not installed: https://github.com/apple/container/releases"
  container system status >/dev/null 2>&1 || container system start
  say "building the check image"
  container build --tag granite-rust:check --file "$here/Dockerfile" "$root"
  mounts=(--volume "$scratch:/scratch" --volume "$cache/registry:/usr/local/cargo/registry" --volume "$cache/git:/usr/local/cargo/git")
  envs=()
  if [ -n "$granite" ]; then
    mounts+=(--volume "$granite:/granite")
    for setting in "${rewrite[@]}"; do envs+=(--env "${setting//GRANITE//granite}"); done
  fi
  # A container's default memory kills rustc on a large crate (wgpu's ash): give the build room.
  sizing=(--memory "${GRANITE_CHECK_MEMORY:-8g}" --cpus "${GRANITE_CHECK_CPUS:-8}")
  granite_cmd() { container run --rm "${sizing[@]}" "${mounts[@]}" ${envs[@]+"${envs[@]}"} granite-rust:check "$@"; }
  inside() { echo "/scratch${1#"$scratch"}"; }
  source=$source_in_container
  schemas=(--schemas /schemas)
else
  say "building granite on this Mac"
  (cd "$root" && cargo build --release -q -p granite-cli)
  native_env=()
  if [ -n "$granite" ]; then
    native_env=(env CARGO_NET_GIT_FETCH_WITH_CLI=true)
    for setting in "${rewrite[@]}"; do native_env+=("${setting//GRANITE/$granite}"); done
  fi
  granite_cmd() { ${native_env[@]+"${native_env[@]}"} "$root/target/release/granite" "$@"; }
  inside() { echo "$1"; }
  source=$source_native
  schemas=(--schemas "$root/Schema" --schemas "$root/containers/schemas")
fi

say "1. installing $agent ($mode)"
granite_cmd agents add "$source" --shelf "$(inside "$scratch/shelf")" > "$scratch/installation.json"
name=$(jq -r .name "$scratch/installation.json")
runner="$(inside "$scratch/shelf")/$name/agent"
granite_cmd check "${schemas[@]}" --schema granite-agent-installation.schema.json "$(inside "$scratch/shelf/$name/installation.json")"
echo "installed $name: $(jq -r '[.entries[].word] | join(", ")' "$scratch/installation.json") · granite $(jq -r .granite "$scratch/installation.json")"

say "2. describing it"
granite_cmd describe "$runner" > "$scratch/entries.json"
count=$(jq length "$scratch/entries.json")
for index in $(seq 0 $((count - 1))); do
  jq ".[$index]" "$scratch/entries.json" > "$scratch/entry-$index.json"
  granite_cmd check "${schemas[@]}" --schema granite-agent-entry.schema.json "$(inside "$scratch/entry-$index.json")"
done
makes=$(jq -c '[.[].makes // [] | .[].kind]' "$scratch/entries.json")

say "3. calling it"
calls=0
for fixture in "$scratch/work"/*.perform.json "$scratch/work"/*.refusal.json; do
  [ -e "$fixture" ] || continue
  calls=$((calls + 1))
  label=$(basename "$fixture" .json)
  want_exit=$(jq -r '.expect.exit // 0' "$fixture")
  set +e
  granite_cmd run "$runner" "$(inside "$fixture")" --directory "$(inside "$scratch/work")" > "$scratch/$label.line"
  code=$?
  set -e
  [ "$code" = "$want_exit" ] || failed "$label: exit $code, expected $want_exit: $(cat "$scratch/$label.line")"
  if [ "$want_exit" = 0 ]; then
    jq .output "$scratch/$label.line" > "$scratch/$label.answer.json"
    granite_cmd check "${schemas[@]}" --schema granite-agent-answer.schema.json "$(inside "$scratch/$label.answer.json")"
    granite_cmd check "${schemas[@]}" --schema craft-agent-output.schema.json "$(inside "$scratch/$label.answer.json")"
    jq -e '.format == "granite-answer/1" and (.agent.word | type == "string")' "$scratch/$label.answer.json" >/dev/null || failed "$label: not stamped"
    for path in $(jq -r '.paths[]' "$scratch/$label.answer.json"); do
      [ $mode = container ] && path="$scratch${path#/scratch}"
      [ -e "$path" ] || failed "$label: $path is named in paths and is not there"
    done
    jq -e --argjson makes "$makes" '[.made[].kind] - $makes == []' "$scratch/$label.answer.json" >/dev/null || failed "$label: made a kind it does not declare"
    jq -e '([.expected[].name] - [.made[].name]) == []' "$scratch/$label.answer.json" >/dev/null || failed "$label: expected something it did not make"
    echo "$label: answered, $(jq -r '[.made[] | "\(.kind) \(.name) (\(.state))"] | join(", ")' "$scratch/$label.answer.json")"
  else
    want_kind=$(jq -r '.expect.kind' "$fixture")
    kind=$(jq -r '.refused.kind // "none"' "$scratch/$label.line")
    [ "$kind" = "$want_kind" ] || failed "$label: refused as $kind, expected $want_kind: $(cat "$scratch/$label.line")"
    echo "$label: refused ($kind): $(jq -r .failed "$scratch/$label.line")"
  fi
done
[ $calls -gt 0 ] || echo "note: $agent keeps no Schema/examples/*.perform.json or *.refusal.json; nothing was called" >&2

if [ -n "$out" ]; then
  mkdir -p "$out"
  cp -R "$scratch/." "$out/"
  echo "kept in $out"
fi
say "check passed: $name, $calls calls ($mode)"
