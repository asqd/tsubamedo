# TsubameDo

HTTP-клиент для работы с API, как Insomnia, но:

- **лёгкий** — ядро на Rust, интерфейс на Tauri вместо Electron;
- **не боится больших ответов** — тело больше 8 МБ сразу уходит во временный файл:
  ответ на 263 МБ скачивается за 0,24 с и занимает 30 МБ памяти;
- **без аккаунта и облака** — коллекции лежат обычными файлами рядом с кодом и живут в git.

Есть приложение для macOS (Tauri) и CLI `tsubamedo` на одном ядре. JSON на 263 МБ
приложение открывает с отступами, сворачиванием и поиском — 28 млн строк листаются сразу.
План и замеры — в [docs/PLAN.md](docs/PLAN.md).

## Сборка

Нужен Rust 1.95 или новее.

Для приложения ещё Node 22 (`nvm use`).

```bash
make test      # тесты ядра
make check     # тесты, clippy и проверка типов интерфейса
make dev       # приложение с горячей перезагрузкой интерфейса
make run       # собрать TsubameDo.app (без подписи) и открыть
make bridge    # интерфейс в браузере на настоящем ядре: http://localhost:1430/bridge.html
make cli       # target/release/tsubamedo
make install   # tsubamedo в ~/.cargo/bin
make icon      # перерисовать иконку из design/make-icon.swift
```

## Устройство

- `core/tsubamedo-core` — всё про запросы: модель, файлы, переменные, отправка, просмотр тела.
- `core/tsubamedo-session` — команды окна за одним вызовом `call`; ими пользуются и приложение,
  и мост для браузера.
- `core/tsubamedo-cli` — `tsubamedo`.
- `apps/desktop` — интерфейс (React + TS), `src-tauri` — оболочка, `bridge` — мост для разработки.

## Коллекция

Коллекция — это папка:

```text
my-api/
  tsubamedo.toml          название и [vars] для всех окружений
  tsubamedo.local.toml    секреты коллекции, в git не попадают
  env/stage.toml          [vars] окружения stage
  env/stage.local.toml    его секреты
  orders/01-create.toml   запросы: папки группируют, имена задают порядок
```

Один запрос — один файл:

```toml
name = "Создать заказ"
method = "POST"
url = "{{base_url}}/orders"

[headers]
Accept = "application/json"
X-Debug = { value = "1", enabled = false }   # выключен, но не потерян

[auth]
type = "bearer"
token = "{{token}}"

[body]
type = "json"
content = '''
{"tour_id": {{tour_id}}, "request_id": "{{$uuid}}"}
'''
```

Когда приложение сохраняет запрос, оно меняет только те строки, которые вы правили:
комментарии, порядок и оформление остаются, а в git видна ровно ваша правка.

Переменные подставляются в порядке силы: коллекция → её `.local` → окружение → его `.local` →
`--var` из командной строки. Встроенные: `{{$uuid}}`, `{{$timestamp}}`, `{{$timestampMs}}`,
`{{$randomInt}}`, `{{$env.ИМЯ}}` (переменная окружения процесса — удобно для секретов в CI).

## CLI

```bash
tsubamedo init my-api                         # новая коллекция с примером
tsubamedo send https://httpbin.org/get -i     # адрес, как curl
tsubamedo send orders/01-create -e stage      # запрос коллекции по имени
tsubamedo send orders/01-create.toml --var token=abc
tsubamedo send https://example.com/big.json -o big.json
tsubamedo run -e stage --fail                 # вся коллекция, код выхода для CI
tsubamedo check                               # все ли переменные заданы во всех окружениях
```

`send` пишет статус и время по фазам (DNS, подключение, TLS, ожидание, загрузка) в stderr,
а тело — в stdout, поэтому его можно сразу передать в `jq`. В терминале JSON форматируется.
Для тестовых стендов с самоподписанным сертификатом есть `-k`.
