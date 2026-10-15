---
title: Проверка выпуска
description: Проверка подписей, контрольных сумм и SBOM выпуска ResticHub перед установкой.
---

Каждый выпуск ResticHub содержит бесключевую подпись для каждого файла и каждого образа. Подпись подтверждает, что файл собрал рабочий процесс выпуска репозитория из помеченного коммита. Закрытого ключа нет, поэтому ключ не может утечь. Проверяйте выпуск перед установкой.

## Что входит в выпуск

- Пакеты агента `restichub-agent-<version>-<os>-<arch>`. В пакетах для Linux лежит статически собранный исполняемый файл.
- Образ управляющего сервера `ghcr.io/fgbm/restichub-cp:<version>` для `linux/amd64` и `linux/arm64`.
- `SHA256SUMS` — сумма SHA-256 каждого файла выпуска.
- Пакет `<file>.sigstore.json` для каждого файла. В нём лежат подпись и сертификат.
- SBOM в формате CycloneDX для каждого пакета (`<package>.cdx.json`) и для образа каждой платформы.

## Что нужно

Установите [cosign](https://docs.sigstore.dev/cosign/system_config/installation/) версии 2 или новее. Задайте ожидаемую идентичность. Это рабочий процесс выпуска на проверяемом теге:

```sh
VERSION=0.1.0
IDENTITY="https://github.com/fgbm/restichub/.github/workflows/release.yml@refs/tags/v${VERSION}"
ISSUER=https://token.actions.githubusercontent.com
```

## Проверка файлов

1. Скачайте файлы выпуска в новый пустой каталог.
2. Проверьте подпись `SHA256SUMS`:

   ```sh
   cosign verify-blob \
     --bundle SHA256SUMS.sigstore.json \
     --certificate-identity "$IDENTITY" \
     --certificate-oidc-issuer "$ISSUER" \
     SHA256SUMS
   ```

   Команда печатает `Verified OK`.
3. Сверьте файлы со списком:

   ```sh
   sha256sum --check --ignore-missing SHA256SUMS
   ```

4. Чтобы проверить один файл, используйте его пакет подписи:

   ```sh
   cosign verify-blob \
     --bundle "restichub-agent-${VERSION}-linux-amd64.tar.gz.sigstore.json" \
     --certificate-identity "$IDENTITY" \
     --certificate-oidc-issuer "$ISSUER" \
     "restichub-agent-${VERSION}-linux-amd64.tar.gz"
   ```

Не устанавливайте файл, если команда завершилась ошибкой. Тег предварительного выпуска, например `v0.1.0-rc.1`, даёт свою идентичность: задайте `VERSION=0.1.0-rc.1`.

## Проверка образа

Проверяйте образ по дайджесту: тег может сместиться, дайджест — нет:

```sh
cosign verify \
  --certificate-identity "$IDENTITY" \
  --certificate-oidc-issuer "$ISSUER" \
  "ghcr.io/fgbm/restichub-cp:${VERSION}"
```

Команда печатает проверенный дайджест. Укажите `ghcr.io/fgbm/restichub-cp@sha256:<digest>` в файле Compose, чтобы закрепить образ.

## Чтение SBOM

Файлы SBOM используют формат CycloneDX 1.5 JSON. SBOM образа также прикреплён к образу как подписанная аттестация:

```sh
cosign verify-attestation --type cyclonedx \
  --certificate-identity "$IDENTITY" \
  --certificate-oidc-issuer "$ISSUER" \
  "ghcr.io/fgbm/restichub-cp:${VERSION}" \
  | jq -r '.payload' | base64 -d | jq '.predicate.components | length'
```

Сопоставьте компоненты с данными об уязвимостях любым инструментом CycloneDX, например `grype sbom:<file>`.

## Windows

Пакет для Windows имеет подпись cosign, как и пакеты для Linux. У файла `restichub-agent.exe` в этом выпуске нет подписи Authenticode, поэтому Windows SmartScreen может предупредить при запуске скачанного файла. Проверьте пакет командами выше до распаковки.
