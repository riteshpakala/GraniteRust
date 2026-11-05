# syntax=docker/dockerfile:1
# The check image: a Rust toolchain, git, and the `granite` command, so a container can install any Rust agent
# repository with GraniteRust's installer (cargo metadata → cargo build → --describe → installation.json) and drive
# its runner as a host would. containers/check.sh builds and runs it with Apple's `container`.
#
# A Linux container has no Metal: an agent with a GPU path draws on its CPU path here, and says so.
FROM docker.io/library/rust:1-slim-bookworm

RUN apt-get update \
 && apt-get install -y --no-install-recommends git ca-certificates jq \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /src/GraniteRust
COPY . .
RUN cargo build --release --locked -p granite-cli \
 && cp target/release/granite /usr/local/bin/granite \
 && mkdir -p /schemas \
 && cp Schema/*.schema.json containers/schemas/*.schema.json /schemas/ \
 && rm -rf target

# Git dependencies are fetched with the git CLI, so a check can point GraniteRust's URL at a mounted clone.
ENV CARGO_NET_GIT_FETCH_WITH_CLI=true
WORKDIR /scratch
ENTRYPOINT ["granite"]
