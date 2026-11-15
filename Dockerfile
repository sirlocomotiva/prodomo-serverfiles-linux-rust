# The `prodomo` server image for `compose.yaml`.
#
# The build stage compiles the one binary from the tracked lockfile. It fetches the locked crates
# from crates.io, so the build needs the network once; nothing is fetched at run time.
#
# The legacy Game data is not in the image: `compose.yaml` mounts `legacy/` read-only, as the
# server reads it in place and never writes it.

FROM docker.io/library/rust:1.85.1-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY protocol protocol
COPY common common
COPY net net
COPY db db
COPY gamedata gamedata
COPY world world
COPY quest quest
COPY prodomo prodomo
COPY parity parity
RUN cargo build --release --locked -p prodomo --bin prodomo

FROM docker.io/library/debian:bookworm-slim
RUN useradd --system --uid 10001 --home-dir /srv/prodomo --shell /usr/sbin/nologin prodomo \
    && install -d -o prodomo -g prodomo /srv/prodomo /srv/prodomo/log
COPY --from=build /src/target/release/prodomo /usr/local/bin/prodomo
USER prodomo
# The example configuration names `legacy/gamedata` and `legacy/sql/gamedata` relative to the
# working directory, and the log folder is `./log`.
WORKDIR /srv/prodomo
EXPOSE 30001 30003 30005 30019
ENTRYPOINT ["prodomo", "--config", "/etc/prodomo/prodomo.toml"]
CMD ["serve"]
