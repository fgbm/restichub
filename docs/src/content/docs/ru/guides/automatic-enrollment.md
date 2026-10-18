---
title: Автоматическая регистрация хостов
description: Как зарегистрировать много хостов из скрипта, Ansible или cloud-init через API, с отдельным токеном регистрации для каждого хоста.
---

Скрипт может регистрировать хосты без веб-интерфейса. Управляющая машина получает токен регистрации через API. Хост получает токен и выполняет `restichub-agent register`. Команда устанавливает агент, регистрирует хост и запускает службу. Эта страница описывает процедуру и даёт примеры для скрипта оболочки, Ansible, cloud-init и PowerShell.

В примерах адрес сервера управления — `https://backup.example.com`. Замените его своим адресом.

## Что нужно

- Токен API с ролью «Администратор». Создайте его в разделе «Настройки» → «Доступ» → «Токены API» (см. [Токены API](/restichub/ru/guides/api-tokens/) и [Справочник API](/restichub/ru/reference/api/#вход-с-токеном-api)).
- Проверенный пакет агента для каждой операционной системы хостов (см. [Проверка выпуска](/restichub/ru/operations/verify-a-release/)).
- `root` на хосте Linux или учётная запись из локальной группы «Администраторы» на хосте Windows.

:::caution
Роль «Администратор» разрешает все операции в организации. Храните токен API в хранилище секретов. Задайте короткий срок действия. Более узкая роль, которая может только создавать токены регистрации, запланирована ([задача #229](https://github.com/fgbm/restichub/issues/229)).
:::

## Шаг 1. Получите токен регистрации

Выполните этот запрос на управляющей машине. Каждый запрос даёт один токен:

```sh
printf 'Authorization: Bearer %s\n' "$RESTICHUB_API_TOKEN" |
  curl -fsS -H @- -X POST https://backup.example.com/api/v1/enrollment-tokens \
    -H 'Content-Type: application/json' \
    -d '{"labels": ["linux", "web"], "ttl_seconds": 900}'
```

Заголовок приходит со стандартного ввода, поэтому токена API нет в аргументах процесса `curl`.

| Поле | Значение |
|---|---|
| `labels` | Необязательное. Сервер управления даёт эти метки хосту, который использует токен. |
| `ttl_seconds` | Необязательное. Срок действия токена: от 60 до 86400 секунд. По умолчанию — 3600. |

Ответ приходит с кодом HTTP 201. Поле `token` содержит значение токена, например `re_<token>`. Сервер управления показывает значение только один раз. Сохраните поле `id`, чтобы потом проверить регистрацию.

Токен регистрирует один хост один раз. Получайте новый токен для каждого хоста. Задайте срок действия немного больше времени установки одного хоста.

## Шаг 2. Зарегистрируйте хост

1. Загрузите пакет агента на хост.
2. Сравните SHA-256 пакета с проверенным файлом `SHA256SUMS` выпуска.
3. Распакуйте пакет в любую папку.
4. Выполните `restichub-agent register --url https://backup.example.com --token-stdin` от имени `root` или администратора.
5. Передайте токен на стандартный ввод команды.

В скрипте используйте `--token-stdin` или `--token-file PATH`. Не используйте `--token`: другие учётные записи могут прочитать аргументы процесса.

Команда выполняет такие шаги:

- Проверяет права и каталоги. Если проверка не прошла, токен остаётся неиспользованным.
- Копирует агент в `/opt/restichub-agent` на Linux или в `%ProgramFiles%\ResticHub\Agent` на Windows.
- Загружает и проверяет restic, если на хосте его нет.
- Регистрирует хост и записывает идентичность в `/etc/restichub-agent` или `%ProgramData%\ResticHub\Agent`.
- Устанавливает и запускает службу `restichub-agent`: модуль systemd на Linux, службу Windows на Windows.
- Ждёт подключения до 60 секунд и выводит сводку.

Код выхода равен 0, только если агент подключён. Все параметры и сводка описаны на странице [Добавление хоста](/restichub/ru/getting-started/add-a-host/#команда-регистрации).

## Примеры

### Скрипт оболочки для одного хоста

Скрипт работает на управляющей машине с `curl`, `jq` и `ssh`. Он копирует проверенный пакет для Linux на хост, получает токен и регистрирует хост:

```sh
#!/bin/sh
# Usage: enroll.sh <host> <package.tar.gz>
set -eu
CP_URL=https://backup.example.com
HOST=$1
PACKAGE=$2

scp "$PACKAGE" "root@$HOST:/tmp/restichub-agent.tar.gz"
ssh "root@$HOST" 'mkdir -p /tmp/restichub-agent &&
  tar -xzf /tmp/restichub-agent.tar.gz -C /tmp/restichub-agent'

printf 'Authorization: Bearer %s\n' "$RESTICHUB_API_TOKEN" |
  curl -fsS -H @- -X POST "$CP_URL/api/v1/enrollment-tokens" \
    -H 'Content-Type: application/json' \
    -d '{"labels": ["linux"], "ttl_seconds": 900}' |
  jq -r .token |
  ssh "root@$HOST" "/tmp/restichub-agent/restichub-agent register --url $CP_URL --token-stdin"
```

Без `jq` замените `jq -r .token` на `sed -n 's/.*"token":"\([^"]*\)".*/\1/p'`.

### Ansible

Первая задача получает токен на управляющей машине. Параметр `no_log: true` не даёт токену API и токену регистрации попасть в вывод. Задачи пропускают хост, у которого уже есть идентичность:

```yaml
- name: Find the agent identity
  ansible.builtin.stat:
    path: /etc/restichub-agent/identity.json
  register: restichub_identity

- name: Create an enrollment token
  ansible.builtin.uri:
    url: "{{ restichub_url }}/api/v1/enrollment-tokens"
    method: POST
    headers:
      Authorization: "Bearer {{ restichub_api_token }}"
    body_format: json
    body:
      labels: ["linux", "web"]
      ttl_seconds: 900
    status_code: 201
  delegate_to: localhost
  become: false
  no_log: true
  register: restichub_enrollment
  when: not restichub_identity.stat.exists

- name: Register the agent
  ansible.builtin.command:
    cmd: "/tmp/restichub-agent/restichub-agent register --url {{ restichub_url }} --token-stdin"
    stdin: "{{ restichub_enrollment.json.token }}"
  become: true
  no_log: true
  when: not restichub_identity.stat.exists
```

Для хоста Windows используйте `ansible.windows.win_stat` с путём `C:\ProgramData\ResticHub\Agent\identity.json`. Затем используйте `ansible.windows.win_command` с тем же параметром `stdin`.

### cloud-init

Получите токен для каждой новой виртуальной машины и запишите его в пользовательские данные. Запишите туда же SHA-256 пакета из проверенного файла `SHA256SUMS`:

```yaml
#cloud-config
write_files:
  - path: /run/restichub/enroll-token
    permissions: "0600"
    content: re_<token>
runcmd:
  - - sh
    - -c
    - |
      set -eu
      VERSION=0.1.0
      PACKAGE=restichub-agent-$VERSION-linux-amd64.tar.gz
      cd /tmp
      curl -fsSLO "https://github.com/fgbm/restichub/releases/download/v$VERSION/$PACKAGE"
      echo "<sha256>  $PACKAGE" | sha256sum -c -
      mkdir -p restichub-agent
      tar -xzf "$PACKAGE" -C restichub-agent
      restichub-agent/restichub-agent register --url https://backup.example.com \
        --token-file /run/restichub/enroll-token
      rm -f /run/restichub/enroll-token
```

Пользовательские данные остаются на виртуальной машине и в облачной платформе. Токен одноразовый и действует недолго, поэтому копия использованного токена не может зарегистрировать хост.

### PowerShell для Windows

1. На хосте загрузите и распакуйте пакет в PowerShell с правами администратора:

   ```powershell
   $Version = '0.1.0'
   $Zip = "restichub-agent-$Version-windows-amd64.zip"
   Invoke-WebRequest "https://github.com/fgbm/restichub/releases/download/v$Version/$Zip" -OutFile "C:\Temp\$Zip"
   $Expected = '<sha256 from the verified SHA256SUMS>'
   if ((Get-FileHash "C:\Temp\$Zip" -Algorithm SHA256).Hash -ne $Expected) { throw 'The checksum is not correct' }
   Expand-Archive "C:\Temp\$Zip" -DestinationPath 'C:\Temp\restichub-agent'
   ```

2. На управляющей машине получите токен и зарегистрируйте хост через удалённое управление PowerShell:

   ```powershell
   $Url = 'https://backup.example.com'
   $Headers = @{ Authorization = "Bearer $env:RESTICHUB_API_TOKEN" }
   $Body = @{ labels = @('windows'); ttl_seconds = 900 } | ConvertTo-Json
   $Created = Invoke-RestMethod -Method Post -Uri "$Url/api/v1/enrollment-tokens" `
     -Headers $Headers -ContentType 'application/json' -Body $Body
   Invoke-Command -ComputerName host01 -ArgumentList $Created.token, $Url -ScriptBlock {
     param($Token, $Url)
     $Token | & 'C:\Temp\restichub-agent\restichub-agent.exe' register --url $Url --token-stdin
   }
   ```

Токен попадает на хост как аргумент блока скрипта, а не как аргумент процесса агента.

## Проверка регистрации

Прочитайте токен по его `id`:

```sh
printf 'Authorization: Bearer %s\n' "$RESTICHUB_API_TOKEN" |
  curl -fsS -H @- "https://backup.example.com/api/v1/enrollment-tokens/<id>" |
  jq '{used_at, used_by_agent_id}'
```

Поле `used_by_agent_id` содержит идентификатор хоста, который использовал токен. Пока хост не использовал токен, поле равно null.

Чтобы получить хосты с меткой, используйте список хостов:

```sh
printf 'Authorization: Bearer %s\n' "$RESTICHUB_API_TOKEN" |
  curl -fsS -H @- "https://backup.example.com/api/v1/agents?label=web&state=online,waiting" |
  jq -r '.items[] | "\(.id) \(.hostname)"'
```

Новый хост находится в состоянии `waiting`, пока агент не откроет поток. Затем хост переходит в состояние `online`.

## Повторная регистрация существующего хоста

При повторной регистрации хост сохраняет идентификатор, планы и метки (см. [Повторная регистрация хоста](/restichub/ru/guides/agent-certificates/)).

1. Отправьте `POST /api/v1/agents/<id>/reenroll` с телом `{}` или `{"ttl_seconds": 600}`.
2. Прочитайте значение токена из поля `token` ответа.
3. На хосте выполните `restichub-agent register --force --url https://backup.example.com --token-stdin`.

```sh
printf 'Authorization: Bearer %s\n' "$RESTICHUB_API_TOKEN" |
  curl -fsS -H @- -X POST "https://backup.example.com/api/v1/agents/<id>/reenroll" \
    -H 'Content-Type: application/json' -d '{}' |
  jq -r .token
```

Ответ содержит те же поля, что и в шаге 1. Поле `replaces_agent_id` содержит идентификатор хоста. Токен не принимает меток. Срок действия токена — от 60 секунд. Предел и значение по умолчанию задаёт [`RESTICHUB_REENROLL_TOKEN_MAX_TTL_SECS`](/restichub/ru/reference/environment-variables/): по умолчанию 3600 секунд, не больше 86400. Новый токен для хоста отменяет прежние неиспользованные токены этого хоста. Для отозванного хоста сервер управления отвечает HTTP 409.

## Ограничения

- Адрес регистрации принимает 10 запросов в минуту с одного IP-адреса и 10 запросов для одного токена. См. [Предел частоты регистрации агентов](/restichub/ru/operations/enrollment-rate-limit/).
- За обратным прокси задайте [`RESTICHUB_TRUSTED_PROXIES`](/restichub/ru/reference/environment-variables/). Иначе все хосты делят счётчик адреса прокси.
- Хосты за одним NAT делят один счётчик. Если таких хостов больше 10, регистрируйте их группами с интервалом в одну минуту.
- Токен истекает после срока действия: от 60 до 86400 секунд.
- Метка содержит от 1 до 63 символов: `a-z`, `0-9` и `-`. Сервер управления удаляет повторяющиеся метки.

## Устранение неполадок

| Признак | Причина | Действие |
|---|---|---|
| `register` останавливается: сервер управления отклонил регистрацию | Хост уже использовал токен, срок токена истёк или администратор отменил токен. | Получите новый токен. Один токен регистрирует один хост. |
| `register` получает HTTP 429 | Слишком много запросов регистрации с одного адреса или для одного токена. | Подождите время из `Retry-After` и снова выполните `register`. Регистрируйте меньше хостов одновременно. |
| Запрос токена получает HTTP 403 | У токена API нет роли «Администратор». | Используйте токен API с ролью «Администратор». |
| Запрос токена получает HTTP 400 | Метка или `ttl_seconds` недопустимы. | Прочитайте имя поля в ошибке. См. [Ограничения](#ограничения). |
| `register` не может подключиться к адресу | Адрес неверный или схема не `https`. | Используйте адрес веб-интерфейса, например `https://backup.example.com`. |
| `register` сообщает об ошибке сертификата | Сертификат сервера управления подписал частный центр сертификации. | Добавьте `--ca-file PATH` с сертификатом этого центра. |
| Команда ждёт стандартный ввод | Токен не пришёл на стандартный ввод. | Передайте токен через конвейер или используйте `--token-file PATH`. |

См. также [Устранение неполадок](/restichub/ru/reference/troubleshooting/#хосты).

## См. также

- [Добавление хоста](/restichub/ru/getting-started/add-a-host/)
- [Сертификаты агента и повторная регистрация](/restichub/ru/guides/agent-certificates/)
- [Токены API](/restichub/ru/guides/api-tokens/)
- [Пакеты агента](/restichub/ru/reference/agent-packages/)
- [Команды и файлы агента](/restichub/ru/reference/agent-commands/)
