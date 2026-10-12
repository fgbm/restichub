# syntax=docker/dockerfile:1

# The build stages and the runtime image of the control plane.
#
# The web UI is built first and embedded into the Rust binary at compile time. The runtime image
# holds the binary only and runs as a non-root user.

# Stage 1: build the web UI.
FROM node:22-bookworm-slim AS ui
WORKDIR /src/ui
COPY ui/package.json ui/package-lock.json ./
RUN npm ci
COPY ui/ ./
RUN npm run build

# Stage 2: build the control plane with the embedded UI.
FROM rust:1.96-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
COPY proto ./proto
# The Rust build embeds `ui/build` at compile time.
COPY --from=ui /src/ui/build ./ui/build
RUN cargo build --release -p restichub-cp

# Stage 3: the runtime image.
FROM debian:bookworm-slim AS runtime
RUN useradd --system --uid 10001 --no-create-home restichub
COPY --from=build /src/target/release/restichub-cp /usr/local/bin/restichub-cp
USER restichub
EXPOSE 8080
ENTRYPOINT ["restichub-cp"]
CMD ["serve"]
