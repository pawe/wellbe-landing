# ---- build ----------------------------------------------------------------
FROM rust:1.94-slim-bookworm AS build
WORKDIR /src

# Dependencies first, against a stub crate, so that editing the site does not
# rebuild the whole dependency tree on every deploy.
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src \
 && echo 'fn main() {}' > src/main.rs \
 && : > src/lib.rs \
 && cargo build --release --locked \
 && rm -rf src \
 # Drop the stub's own artifacts, so the real build cannot be mistaken for
 # already done. Deleting these is more reliable than hoping COPY gives the
 # sources a newer mtime than the stub build.
 && rm -f target/release/wellbe-landing \
 && rm -f target/release/deps/wellbe_landing* \
 && rm -f target/release/.fingerprint/wellbe-landing-*/*

# Askama compiles the templates into the binary and sqlx embeds the migrations,
# so both have to be present at build time — not only at runtime.
COPY templates ./templates
COPY migrations ./migrations
COPY src ./src
RUN cargo build --release --locked

# ---- run ------------------------------------------------------------------
FROM debian:bookworm-slim AS runtime
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --create-home --uid 10001 wellbe

WORKDIR /app
COPY --from=build /src/target/release/wellbe-landing /usr/local/bin/wellbe-landing
# Unlike the templates, these are read from disk at runtime.
COPY static ./static

USER wellbe
EXPOSE 8080
ENV BIND=0.0.0.0:8080

# Healthy means "can reach the database", not merely "the process is alive":
# a landing page that cannot record a signup is down. Done in-process because
# this image deliberately has no curl to call /health with.
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD ["/usr/local/bin/wellbe-landing", "--health-check"]

CMD ["wellbe-landing"]
