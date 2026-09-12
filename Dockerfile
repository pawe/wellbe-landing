# ---- build ----------------------------------------------------------------
FROM rust:1.94-slim-bookworm AS build
WORKDIR /src

# Dependencies first, so that editing the site does not rebuild the world.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src \
 && echo 'fn main() {}' > src/main.rs \
 && echo '' > src/lib.rs \
 && cargo build --release \
 && rm -rf src

# Templates are compiled into the binary by askama and migrations are embedded
# by sqlx, so both have to be here at build time — not just at runtime.
COPY templates ./templates
COPY migrations ./migrations
COPY src ./src

# Cargo skips the rebuild unless it sees the sources as newer than the stubs.
RUN touch src/main.rs src/lib.rs && cargo build --release

# ---- run ------------------------------------------------------------------
FROM debian:bookworm-slim AS runtime
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --create-home --uid 10001 wellbe

WORKDIR /app
COPY --from=build /src/target/release/wellbe-landing /usr/local/bin/wellbe-landing
# Served from disk at runtime, so unlike the templates these must be copied in.
COPY static ./static

USER wellbe
EXPOSE 8080
ENV BIND=0.0.0.0:8080

# The container is healthy when it can reach the database, not merely when the
# process is alive — a landing page that cannot record a signup is down.
HEALTHCHECK --interval=30s --timeout=3s --start-period=10s --retries=3 \
  CMD ["/usr/local/bin/wellbe-landing", "--health-check"]

CMD ["wellbe-landing"]
