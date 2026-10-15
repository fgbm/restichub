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

Установите [cosign](https://docs.sigstore.dev/cosign/system_config/installation/) версии 3.0 или новее. Версия 2 не находит подписи образа. Задайте ожидаемую идентичность. Это рабочий процесс выпуска на проверяемом теге:

<!-- ci-run -->
```sh
VERSION=0.1.0
IDENTITY="https://github.com/fgbm/restichub/.github/workflows/release.yml@refs/tags/v${VERSION}"
ISSUER=https://token.actions.githubusercontent.com
```

## Проверка файлов

1. Скачайте файлы выпуска в новый пустой каталог.
2. Проверьте подпись `SHA256SUMS`:

   <!-- ci-run -->
   ```sh
   cosign verify-blob \
     --bundle SHA256SUMS.sigstore.json \
     --certificate-identity "$IDENTITY" \
     --certificate-oidc-issuer "$ISSUER" \
     SHA256SUMS
   ```

   Команда печатает `Verified OK`.
3. Сверьте файлы со списком:

   <!-- ci-run -->
   ```sh
   sha256sum --check --ignore-missing SHA256SUMS
   ```

4. Чтобы проверить один файл, используйте его пакет подписи:

   <!-- ci-run -->
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

<!-- ci-run -->
```sh
IMAGE=ghcr.io/fgbm/restichub-cp
DIGEST=$(docker buildx imagetools inspect "$IMAGE:${VERSION}" --format '{{json .Manifest}}' | jq -r .digest)
cosign verify \
  --certificate-identity "$IDENTITY" \
  --certificate-oidc-issuer "$ISSUER" \
  "$IMAGE@$DIGEST"
```

Команда проверяет подпись дайджеста, на который указывает тег. Укажите в файле Compose `ghcr.io/fgbm/restichub-cp@sha256:<digest>` с этим дайджестом, а не тег. Эталонный стек берёт ссылку из `RESTICHUB_IMAGE`.

## Чтение SBOM

Файлы SBOM используют формат CycloneDX 1.5 JSON. SBOM образа каждой платформы также прикреплён к образу платформы как подписанная аттестация. Сначала прочитайте дайджест платформы:

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

Сопоставьте компоненты с данными об уязвимостях любым инструментом CycloneDX, например `grype sbom:<file>`.

## Воспроизведение пакета для Linux

Пакеты агента для Linux воспроизводимы. Сборка того же коммита тем же набором инструментов даёт тот же архив байт в байт. Рабочий процесс задаёт `SOURCE_DATE_EPOCH` по времени коммита, заменяет пути сборки через `--remap-path-prefix` и пишет архив с отсортированными записями, фиксированным владельцем и фиксированным временем. Файлы SBOM содержат время создания. Пакет для Windows и образы на это свойство не проверяются.

## Windows

Пакет для Windows имеет подпись cosign, как и пакеты для Linux. У файла `restichub-agent.exe` в этом выпуске нет подписи Authenticode, поэтому Windows SmartScreen может предупредить при запуске скачанного файла. Проверьте пакет командами выше до распаковки.
