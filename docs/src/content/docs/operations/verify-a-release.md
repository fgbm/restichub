---
title: Verify a release
description: Check the signatures, the checksums and the SBOM of a ResticHub release before you install it.
---

Each release of ResticHub carries a keyless signature for each file and for each image. The signature proves that the release workflow of the repository built the file from the tagged commit. No private key exists, so no key can leak. Verify a release before you install it.

## What a release holds

- The agent packages `restichub-agent-<version>-<os>-<arch>`. Linux packages hold a static binary.
- The control plane image `ghcr.io/fgbm/restichub-cp:<version>` for `linux/amd64` and `linux/arm64`.
- `SHA256SUMS`: the SHA-256 of each file of the release.
- A `<file>.sigstore.json` bundle for each file. It holds the signature and the certificate.
- A CycloneDX SBOM for each package (`<package>.cdx.json`) and for each platform image.

## What you need

Install [cosign](https://docs.sigstore.dev/cosign/system_config/installation/) version 3.0 or later. Version 2 does not find the signatures of the image. Set the identity that you expect. The identity is the release workflow at the tag that you verify:

<!-- ci-run -->
```sh
VERSION=0.1.0
IDENTITY="https://github.com/fgbm/restichub/.github/workflows/release.yml@refs/tags/v${VERSION}"
ISSUER=https://token.actions.githubusercontent.com
```

## Verify the files

1. Download the files of the release into a new empty directory.
2. Verify the signature of `SHA256SUMS`:

   <!-- ci-run -->
   ```sh
   cosign verify-blob \
     --bundle SHA256SUMS.sigstore.json \
     --certificate-identity "$IDENTITY" \
     --certificate-oidc-issuer "$ISSUER" \
     SHA256SUMS
   ```

   The command prints `Verified OK`.
3. Check the files against the list:

   <!-- ci-run -->
   ```sh
   sha256sum --check --ignore-missing SHA256SUMS
   ```

4. To verify one file alone, use its bundle:

   <!-- ci-run -->
   ```sh
   cosign verify-blob \
     --bundle "restichub-agent-${VERSION}-linux-amd64.tar.gz.sigstore.json" \
     --certificate-identity "$IDENTITY" \
     --certificate-oidc-issuer "$ISSUER" \
     "restichub-agent-${VERSION}-linux-amd64.tar.gz"
   ```

Do not install a file when a command fails. A pre-release tag, for example `v0.1.0-rc.1`, uses its own identity: set `VERSION` to `0.1.0-rc.1`.

## Verify the image

Verify the image by its digest. A tag can move, and a digest cannot:

<!-- ci-run -->
```sh
IMAGE=ghcr.io/fgbm/restichub-cp
DIGEST=$(docker buildx imagetools inspect "$IMAGE:${VERSION}" --format '{{json .Manifest}}' | jq -r .digest)
cosign verify \
  --certificate-identity "$IDENTITY" \
  --certificate-oidc-issuer "$ISSUER" \
  "$IMAGE@$DIGEST"
```

The command checks the signature of the digest that the tag points to. Use `ghcr.io/fgbm/restichub-cp@sha256:<digest>` with that digest, and not the tag, in your Compose file. The reference stack takes the reference in `RESTICHUB_IMAGE`.

## Read the SBOM

The SBOM files use the CycloneDX 1.5 JSON format. The SBOM of each platform image is also attached to the platform image as a signed attestation. Read the digest of the platform first:

<!-- ci-run -->
```sh
PLATFORM_DIGEST=$(docker buildx imagetools inspect "$IMAGE:${VERSION}" --format '{{json .Manifest}}' \
  | jq -r '.manifests[] | select(.platform.architecture=="amd64") | .digest')
cosign verify-attestation --type cyclonedx \
  --certificate-identity "$IDENTITY" \
  --certificate-oidc-issuer "$ISSUER" \
  "$IMAGE@$PLATFORM_DIGEST" \
  | jq -r '.payload' | base64 -d | jq '.predicate.components | length'
```

Use any CycloneDX tool, for example `grype sbom:<file>`, to match the components against vulnerability data.

## Reproduce a Linux package

The Linux agent packages are reproducible. A build of the same commit with the same toolchain gives the same archive bytes. The workflow sets `SOURCE_DATE_EPOCH` to the commit time and maps the build paths with `--remap-path-prefix`. It writes each archive with sorted entries, a fixed owner and a fixed time. The SBOM files hold their generation time. The Windows package, the SBOM files and the images are not byte-reproducible, and the workflow does not check them. A reproducible Windows package and reproducible images are planned for the 1.0 release.

To compare a package with the release, build the same commit on Ubuntu 24.04 (`x86_64`) with the `musl-tools` package, the Rust toolchain of the file `rust-toolchain.toml` (1.96) and `rustup`. The commands below build the `amd64` package. Set `VERSION` to the version of the release.

```sh
VERSION=0.1.0
git clone https://github.com/fgbm/restichub.git
cd restichub
git checkout "v$VERSION"
sudo apt-get install -y --no-install-recommends musl-tools
rustup target add x86_64-unknown-linux-musl
export SOURCE_DATE_EPOCH="$(git log -1 --format=%ct)"
export RUSTFLAGS="--remap-path-prefix=$PWD=/build/src --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/build/cargo --remap-path-prefix=${RUSTUP_HOME:-$HOME/.rustup}=/build/rustup"
RESTICHUB_BUILD_VERSION="$(git rev-parse --short=7 HEAD)" RESTICHUB_RELEASE_VERSION="$VERSION" \
  cargo build --release --locked -p restichub-agent --target x86_64-unknown-linux-musl
packaging/assemble-agent-package.sh target/x86_64-unknown-linux-musl/release/restichub-agent linux amd64 "$VERSION" out
sha256sum out/*.tar.gz
```

The hash must equal the line of the package in the signed `SHA256SUMS` file of the release. For the `arm64` package, build on an `arm64` host with the target `aarch64-unknown-linux-musl`. A different `musl-tools` or toolchain version gives a different binary.

## Windows

The Windows package has a cosign signature like the Linux packages. The `restichub-agent.exe` file has no Authenticode signature in this release, so Windows SmartScreen can warn when you run it from a download. Authenticode signing is planned for the 1.0 release. Verify the package with the commands above before you unpack it.
