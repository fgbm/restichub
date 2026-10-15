---
title: Справочник API
description: Как войти в API, какие заголовки нужны и как прочитать документ OpenAPI.
---

Сервер управления предоставляет REST API по адресу `/api/v1`. Веб-интерфейс работает через тот же API. На этой странице показано, как пройти аутентификацию. Документ OpenAPI перечисляет все маршруты, поля и ошибки.

## Документ OpenAPI

Сервер управления отдаёт документ OpenAPI по запросу `GET /api/v1/openapi.json`. Для этого маршрута учётные данные не нужны.

```sh
curl -s https://<control-plane>/api/v1/openapi.json -o restichub-openapi.json
```

Откройте файл в просмотрщике OpenAPI или в генераторе кода. В документе описаны две схемы защиты: `cookie` для сессии и `bearer` для токена API.

## Вход с токеном API

Для скрипта используйте токен API. Токен принадлежит одной организации и имеет одну роль.

1. Откройте раздел «Токены API» в веб-интерфейсе.
2. Нажмите «Создать токен». Введите имя, роль и, по желанию, срок действия.
3. Скопируйте значение токена. Сервер управления показывает его только один раз.

Значение токена начинается с `rh_`. Передайте его в заголовке `Authorization`:

```sh
curl -s https://<control-plane>/api/v1/plans \
  -H "Authorization: Bearer rh_<token>"
```

Для запроса с токеном заголовок CSRF не нужен. Роль токена ограничивает маршруты. Если роль не допускает маршрут, сервер отвечает HTTP 403.

Чтобы создать токен через API, используйте сессию или токен с ролью администратора:

```sh
curl -s -X POST https://<control-plane>/api/v1/api-tokens \
  -H "Authorization: Bearer rh_<admin-token>" \
  -H "Content-Type: application/json" \
  -d '{"name": "ci", "role": "viewer", "expires_at": "2027-01-01T00:00:00Z"}'
```

В ответе есть поле `token`. Повторно сервер управления его не возвращает.

## Вход с сессией

Веб-интерфейс использует cookie сессии. Скрипт может сделать то же самое:

```sh
curl -s -c cookies.txt -X POST https://<control-plane>/api/v1/auth/sign-in \
  -H "X-ResticHub-CSRF: 1" \
  -H "Content-Type: application/json" \
  -d '{"username": "alice", "password": "<password>"}'
```

Сервер управления устанавливает cookie `__Host-restichub_session`. Запрос, который изменяет данные, с этим cookie должен передавать заголовок `X-ResticHub-CSRF: 1`. Запрос входа тоже требует этот заголовок. Без заголовка сервер отвечает HTTP 403.

```sh
curl -s -b cookies.txt -X POST https://<control-plane>/api/v1/plans/<plan-id>/run \
  -H "X-ResticHub-CSRF: 1"
```

Не передавайте cookie и токен в одном запросе. Такой запрос получает ответ HTTP 400.

## Ошибки

Ответ с ошибкой использует тип данных из RFC 9457. В теле есть такие поля:

| Поле | Значение |
|---|---|
| `status` | Статус HTTP. |
| `code` | Устойчивый код в формате snake_case. Веб-интерфейс переводит его. |
| `title` | Короткий текст на английском языке. |
| `request_id` | Идентификатор запроса. Сообщите его администратору сервера управления. |
| `field`, `reason` | Неверное поле и причина. Их задаёт только код `invalid_argument`. |

## Списки

Маршрут списка возвращает `items` и `next_cursor`. Чтобы получить следующую страницу, передайте значение `next_cursor` в параметре `cursor`. Параметр `limit` задаёт размер страницы.

```sh
curl -s "https://<control-plane>/api/v1/plans?limit=50" \
  -H "Authorization: Bearer rh_<token>"
```

## Связанные страницы

- [Метрики Prometheus](/restichub/ru/operations/metrics/)
- [Переменные окружения](/restichub/ru/operations/environment-variables/)
