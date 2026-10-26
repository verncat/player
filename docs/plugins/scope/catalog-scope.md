# Catalog scope: репозиторий плагинов

Точка входа: [readme](../../plugin/readme.md). Реализация: `catalog.rs`,
`PluginCatalog.vue`, `catalog.ts`. JSON каталога хранится в репозитории Player:
[plugins/catalog.json](../../../plugins/catalog.json).

## Получение списка

Основной адрес:
`https://raw.githubusercontent.com/verncat/player/main/plugins/catalog.json`.
Коннектор, Git-клиент и GitHub OAuth не нужны. Поддержаны desktop и Android.

Settings → Plugins показывает каталог с поиском по названию, ID, описанию,
автору и тегам. По умолчанию новые записи идут первыми; при одинаковой дате —
по названию. Карточка содержит картинку, автора, ссылку на репозиторий, даты,
версию, платформы, теги и отметку установленного пакета. Для неподдерживаемой
платформы и включённого установленного плагина действие установки недоступно.

`plugins_catalog({refresh:false})` сразу возвращает сохранённый список или копию
JSON из сборки. Затем UI вызывает `refresh:true` в фоне. Кнопка Refresh catalog
повторяет запрос. Успешный ответ сохраняется в `<app_data_dir>/plugin-system/catalog.json`
с backup `catalog.bak`. Ошибка сети, 404 или невалидный ответ сохраняют прежний
список и показываются рядом с каталогом; локальная установка остаётся доступной.
Кеш привязан к URL источника и заново валидируется при чтении.

Первый ответ содержит `source: remote | cache | bundled`, `sourceUrl`, текущую
`platform`, объект `catalog` и необязательное `warning`. До успешного получения
данных из сети UI показывает Offline catalog. Загрузка списка не скачивает код
перечисленных плагинов и не включает их.

## JSON schema 1

```json
{
  "schemaVersion": 1,
  "updatedAt": "2026-10-27",
  "plugins": [
    {
      "id": "author.my-plugin",
      "name": "My plugin",
      "description": "What this extension adds to Player.",
      "repository": "https://github.com/author/my-plugin",
      "installUrl": "https://github.com/author/my-plugin",
      "addedAt": "2026-10-27",
      "updatedAt": "2026-10-27",
      "version": "1.0.0",
      "author": {"name": "Author", "url": "https://github.com/author"},
      "image": "https://example.org/plugin.png",
      "platforms": ["macos", "windows", "linux", "android"],
      "tags": ["lyrics", "metadata"]
    }
  ]
}
```

Обязательны глобальные `schemaVersion`, `updatedAt`, `plugins`; у записи —
`id`, `name`, `description`, `repository`, `addedAt`, `platforms`. Остальные
поля необязательны, `tags` по умолчанию пуст. `author`, `image`, имя, версия,
платформы и ID используют правила [package scope](package-scope.md).
Зарезервированные `player.*` не включаются в каталог внешних пакетов:
Discord RPC и Soulseek уже представлены установленными встроенными плагинами.

`repository` и `installUrl` — HTTPS без встроенных credentials. Если `installUrl`
нет, используется `repository`. Для monorepo задайте installUrl папки с
plugin.json/main.js; для готового пакета — прямую HTTPS-ссылку на `.player-plugin`.
В `repository` можно оставить ссылку на основной репозиторий автора.

Даты — календарные `YYYY-MM-DD`. `addedAt` не позже глобального `updatedAt`;
дата изменения записи не раньше её добавления и не позже глобальной даты.
Максимум 256 записей, ID уникальны, до 12 тегов по 40 символов на запись.
Размер JSON — до 512 KiB. Неизвестные поля или версия схемы отклоняются целиком;
повреждённый ответ не заменяет действующий кеш.

## Установка

Review plugin передаёт installUrl/repository в существующий HTTPS downloader
и добавляет `expectedId`. Rust сравнивает ID полученного пакета с выбранной
записью. Несовпадение отклоняется до preview. Затем используется обычный
просмотр actual manifest, source commit, fingerprint и permissions; установка
происходит только после подтверждения. Пакет сохраняется отключённым.

Сведения каталога могут отставать от репозитория. Каталог не выдаёт права и
не подменяет manifest загруженного пакета; заявленная в нём версия не является
проверкой наличия обновления. При обновлении включённый плагин сначала отключают.
APK-плагины пока устанавливаются средствами Android и подключаются через
Find APK plugins, а не через этот каталог JSON-пакетов.

## Публикация и разработка

Чтобы добавить плагин, добавьте запись в `plugins/catalog.json` и обновите
глобальную дату. Добавляйте только существующий пакет с соответствующим ID.
Новые записи в удалённом JSON появляются при обновлении каталога без новой
сборки приложения; смена схемы требует поддержки нового schemaVersion.

Текущий пример Hello Player ссылается на папку ветки `feat/plugins`, где он
уже опубликован. После переноса примера в main можно поменять installUrl в JSON.
Сам индекс по основному адресу появится после публикации файла в main. До этого
сборка работает с вложенной копией; для проверки удалённого индекса ветки:

```sh
PLAYER_PLUGIN_CATALOG_URL="https://raw.githubusercontent.com/verncat/player/feat/plugins/plugins/catalog.json" pnpm tauri dev
```

Переменная задаётся при сборке Rust; UI не меняет источник каталога произвольно.
Проверки покрывают формат и календарные даты, дубликаты/лимиты, недопустимые
URL и изображения, кеш/backup, отказ сети и повреждённый ответ, фильтрацию,
сортировку и несовпадение ID устанавливаемого пакета.
