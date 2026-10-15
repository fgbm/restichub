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

Install [cosign](https://docs.sigstore.dev/cosign/system_config/installation/) version 2 or later. Set the identity that you expect. The identity is the release workflow at the tag that you verify:

```sh
VERSION=0.1.0
IDENTITY="https://github.com/fgbm/restichub/.github/workflows/release.yml@refs/tags/v${VERSION}"
ISSUER=https://token.actions.githubusercontent.com
```

## Verify the files

1. Download the files of the release into a new empty directory.
2. Verify the signature of `SHA256SUMS`:

   ```sh
   cosign verify-blob \
     --bundle SHA256SUMS.sigstore.json \
     --certificate-identity "$IDENTITY" \
     --certificate-oidc-issuer "$ISSUER" \
     SHA256SUMS
   ```

   The command prints `Verified OK`.
3. Check the files against the list:

   ```sh
   sha256sum --check --ignore-missing SHA256SUMS
   ```

4. To verify one file alone, use its bundle:

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

```sh
cosign verify \
  --certificate-identity "$IDENTITY" \
  --certificate-oidc-issuer "$ISSUER" \
  "ghcr.io/fgbm/restichub-cp:${VERSION}"
```

The command prints the digest that it verified. Use `ghcr.io/fgbm/restichub-cp@sha256:<digest>` in your Compose file to pin it.

## Read the SBOM

The SBOM files use the CycloneDX 1.5 JSON format. The SBOM of an image is also attached to the image as a signed attestation:

```sh
cosign verify-attestation --type cyclonedx \
  --certificate-identity "$IDENTITY" \
  --certificate-oidc-issuer "$ISSUER" \
  "ghcr.io/fgbm/restichub-cp:${VERSION}" \
  | jq -r '.payload' | base64 -d | jq '.predicate.components | length'
```

Use any CycloneDX tool, for example `grype sbom:<file>`, to match the components against vulnerability data.

## Windows

The Windows package has a cosign signature like the Linux packages. The `restichub-agent.exe` file has no Authenticode signature in this release, so Windows SmartScreen can warn when you run it from a download. Verify the package with the commands above before you unpack it.
