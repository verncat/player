# Storage / security scope

Зависимости: [package](package-scope.md), [runtime](runtime-scope.md).
Реализация: native store, secrets providers и capability gateway.

## Устойчивое хранение

Корень: `<app_data_dir>/plugin-system`, вне сканируемой музыкальной библиотеки.
`registry.json` содержит внешние пакеты и записи enabled/generation/settings и источник установки (URL/repository/revision).
Запись идёт через временный файл, sync и backup `registry.bak`; на Unix новые
файлы создаются с mode 0600. Старое содержимое сохраняется для восстановления.
Staging расположен в `staging/<pluginId>`, копии импорта — в `imports/<pluginId>`.
Музыкальная синхронизация не переносит пакеты, настройки или секреты.

`settings.get` → `{revision,values}`; values — JSON object.
`settings.update({revision,values})` заменяет только собственные настройки.
CAS отвергает устаревшую revision с CONFLICT. Объект ограничен 64 KiB.
Плагин сам версионирует свою схему внутри values и выполняет преобразования;
хост не исполняет произвольные migration hooks при установке пакета.

`secrets.has/get/set/delete` принимают `key`; set также `value`.
Key — локальный ID, до 64 символов `[a-z0-9-]`. Полный ключ строит хост из
идентичности Worker. Секреты не попадают в registry/settings и snapshots.
Password input получает только пустое начальное значение; новое значение
передаётся для сохранения. `get` доступен только к секретам своего плагина.

`has` проверяет сохранённый признак наличия ключа в реестре, не читая пароль
и не вызывая системный диалог. Этот признак обновляется после успешных set/delete
и фактического чтения; удаление секрета вне Player обнаруживается при чтении.
Результат get (включая отсутствие и отказ доступа) кешируется только в RAM на
время активации. Одновременные чтения объединяются в один вызов provider.
Set/delete обновляют кеш только после успеха. Переход enable/disable очищает
кеш плагина; повторное включение позволяет явно повторить доступ после отказа.
OS-вызовы из plugin gateway выполняются в blocking pool. Отказ доступа не
подменяется пустым паролем или legacy-значением. Проверка Soulseek status
использует metadata и не читает Keychain/не создаёт сетевой client.

| Платформа | Provider |
| --- | --- |
| macOS | Keychain generic password, service `dev.verncat.player.plugins` |
| Windows | Credential Manager, generic credential текущего пользователя |
| Linux | Secret Service через secret-tool; provider/session должны быть доступны |
| Android | AndroidKeyStore AES/GCM, encrypted private SharedPreferences |

При отказе provider новые пароли не пишутся открытым текстом. Миграция старого
пароля Soulseek откладывается, исходная запись не уничтожается. После успешного
переноса старая строка очищается. Это не стирает исторические backup/WAL-файлы.

## Права API 1

| Permission | Выдаваемый доступ |
| --- | --- |
| ui | Объявленные страницы, настройки, поиск, действия, виджеты, собственная навигация |
| playback.read | Playback snapshots |
| playback.control | Play своего проверенного sourceToken и pause/resume/stop/seek текущего transport |
| library.import / library.replace | Импорт / замена через хост |
| settings | Собственный JSON settings с CAS |
| secrets | Собственные credential entries |
| network.http | HTTP request/download хоста |
| native.discord / native.soulseek | Зарезервированные адаптеры встроенных пакетов |
| native.android | Вызовы установленного APK; unrestricted native-код с Android-разрешениями Player |

`http.request({url,method?,headers?,body?})` поддерживает GET/POST/PUT/PATCH/
DELETE/HEAD; возвращает `{status,body}`. Тело запроса/ответа до 1 MiB, timeout
30 секунд. HTTP body — текст. Секреты в URL и сообщения журнала не включать.

Проверка ID, generation, enabled, платформы и permission выполняется на
native-стороне. Source handles дополнительно проверяют владельца и поколение.
Management-команды доступны оболочке; Worker не получает общий invoke.
Установка/обновление с новым набором прав снова показывает эти права пользователю.

## Граница доверия

Пакет — исполняемый JavaScript доверенного автора. Worker отделяет его от DOM
и Tauri bridge основного окна, но стандартные сетевые возможности Worker
не объявляются изолированными разрешением network.http. Это право ограничивает
API хоста, а не служит полной browser/OS sandbox. Нативные адаптеры доверены.
APK-расширение с `native.android` выполняется в процессе Player: остальные
permissions не ограничивают его доступ к памяти, сети, файлам или credentials.
Сертификат и hash фиксируют просмотренную версию; они не удостоверяют личность
автора. Обновление требует нового preview, загрузка изменённой `.so` — restart.
Подробности: [Android native scope](android-native-scope.md).

UI — только проверяемая схема: известные узлы, ограничения глубины/размеров,
текстовая интерполяция, отсутствие remote HTML/eval. Сведения о разрешениях
Tauri на уровне окна: [capabilities](https://tauri.app/security/capabilities/).
Они не изолируют отдельные Vue-компоненты одного WebView.

Приёмка: вызов без permission, чужой/старый source token, spoof plugin ID,
settings CAS, недоступный credential provider, отсутствие plaintext-пароля в
registry/логах, malformed UI/package и сохранение музыки при uninstall.
