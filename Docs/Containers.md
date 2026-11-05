# Containers

`containers/` is where an agent repository is installed and checked the way a host will install it, inside a Linux
container (Apple's `container`, arm64) or on this Mac.

```bash
containers/check.sh <agent folder | git URL[@revision]> [--native] [--granite <GraniteRust clone>] [--out <dir>]
```

| Step | What is held |
| --- | --- |
| 1. Install | GraniteRust's installer clones the repository's last commit, reads it with `cargo metadata`, builds its runner in release, describes it; `installation.json` validates against `granite-agent-installation.schema.json` |
| 2. Describe | every entry validates against `granite-agent-entry.schema.json` |
| 3. Call | every `Schema/examples/*.perform.json` is answered (exit 0), the answer valid against `granite-agent-answer.schema.json` and Craft's `craft-agent-output.schema.json`, stamped, every path written, every `made` kind declared, every `expected` made; every `*.refusal.json` is refused with its kind |

The examples folder is the call's folder, so a fixture names its files relative to it:

```json
{"perform": {"word": "comic", "input": {"script": "hashmap.comic.json", "seed": 7}, "model": false}, "expect": {"exit": 0}}
{"perform": {"word": "comic", "input": {"panels": 9}, "model": false}, "expect": {"exit": 3, "kind": "input"}}
```

## The image

`containers/Dockerfile`: `rust:1-slim-bookworm`, git and jq, the `granite` command, and the schemas in `/schemas`. The
cargo registry and git checkouts are mounted from `~/.cache/granite-rust`, so a second check downloads nothing. Each container gets 8 GB and 8 CPUs (`GRANITE_CHECK_MEMORY`,
`GRANITE_CHECK_CPUS`): with the default, the kernel kills rustc on a crate as large as wgpu's `ash`. A
Linux container has no Metal: an agent with a GPU path draws on its CPU path inside, and its answer should say so.

## Before GraniteRust is pushed

`--granite <clone>` points GraniteRust's GitHub URL at a clone, for the check only, with git's own URL rewriting
(`GIT_CONFIG_*`, `url.<clone>.insteadOf`) and cargo fetching with the git CLI; in a container the clone is mounted at
`/granite`. Cargo.lock keeps the GitHub URL at the clone's commits, so once they are pushed the same lock reads from
GitHub.

## Installing Apple's container

The signed package from github.com/apple/container/releases installs into `/usr/local` with an administrator's
password. Without one, its payload runs from any folder:

```bash
pkgutil --expand-full container-<version>-installer-signed.pkg expanded
ditto expanded/Payload ~/.local/apple-container/<version>
~/.local/apple-container/<version>/bin/container system start --install-root ~/.local/apple-container/<version> --enable-kernel-install
```

`container system stop` ends its services; removing the folder removes it.

## GraniteRust's own suite on Linux

`scripts/conformance-linux.sh` builds `containers/Dockerfile.linux` and runs `scripts/check.sh` inside it, the
installer's test included: the process group, `wait4`'s units (`ru_maxrss` is kilobytes on Linux) and `/tmp` are the
platform code it proves.
