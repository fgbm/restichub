# syntax=docker/dockerfile:1

# The build stages and the runtime image of the control plane.
#
# The web UI is built first and embedded into the Rust binary at compile time. The runtime image
# holds the binary only and runs as a non-root user. The base images are pinned by digest.

# Stage 1: build the web UI.
FROM node:22-bookworm-slim@sha256:c3de60bf2f9dd0ac6370e6117950ff62d6e339527e7472301c9c78a017978392 AS ui
WORKDIR /src/ui
COPY ui/package.json ui/package-lock.json ./
RUN npm ci
COPY ui/ ./
RUN npm run build

# Stage 2: build the control plane with the embedded UI.
FROM rust:1.96-bookworm@sha256:a339861ae23e9abb272cea45dfafde21760d2ce6577a70f8a926153677902663 AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
COPY proto ./proto
# The Rust build embeds `ui/build` at compile time.
COPY --from=ui /src/ui/build ./ui/build
# CI passes the short revision as the build part of the version, and for a release the full
# version with its pre-release suffix.
ARG RESTICHUB_BUILD_VERSION
ARG RESTICHUB_RELEASE_VERSION
RUN cargo build --release --locked -p restichub-cp

# Stage 3: the runtime image.
#
# The runtime needs no CA certificate bundle: sqlx uses rustls with the bundled `webpki-roots`.
FROM debian:bookworm-slim@sha256:7c7b2c966bc9ee8cedfeef67e0e279108992c77681fa595db4a9d65c06ccc587 AS runtime
RUN useradd --system --uid 10001 --no-create-home restichub
COPY --from=build /src/target/release/restichub-cp /usr/local/bin/restichub-cp
USER restichub
EXPOSE 8080
ENTRYPOINT ["restichub-cp"]
CMD ["serve"]
