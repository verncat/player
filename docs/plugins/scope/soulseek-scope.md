# Soulseek scope

Пакет: `plugins/soulseek`; адаптер: `src-tauri/src/plugins/soulseek.rs`.
Feature `soulseek`. Зависимость native-кода — vendored
`src-tauri/vendor/soulseek-rs-lib`; `soulseek-rs-repo` не используется сборкой.
Платформы: macOS/Windows/Linux/Android. По умолчанию выключен.

## Contributions и состояние

| ID | kind | Поведение |
| --- | --- | --- |
| connection | settings | Username, смена пароля, ошибки сохранения |
| search | search | Явный сетевой поиск, качество/размер/пользователь, preview/save |
| replacement | page | Внутренняя страница выбора замены для переданного Track; `showInMenu: false` |
| find-replacement | track-action | Открывает replacement с выбранным Track |
| buffer | player-widget | Количество текущих native-передач |

Username находится в собственных settings, password — в secrets под `password`.
Сохранение формы не перезаписывает настройки устройства. Пустое поле нового
пароля сохраняет старый секрет. Ошибка provider остаётся видимой в настройках.

## Native API пакета

Все вызовы требуют native.soulseek и привязаны к его generation:

| Метод | Аргументы / результат |
| --- | --- |
| soulseek.status | enabled/configured/session status |
| soulseek.search | `{query}` → массив сетевых результатов |
| soulseek.cover | `{request:{username,coverFilename,coverSize}}` → cover hash/null |
| soulseek.preview | `{request:{username,filename,size,coverFilename?,coverSize?}}` → transfer ID |
| soulseek.cancelPreview | `{request:{username,filename}}` → bool |
| soulseek.prepare | `{username,filename,resource}` → sourceToken |
| soulseek.download | Тот же request; обычная native-загрузка с размещением в библиотеке |

Основной Worker использует preview как единую передачу для прослушивания
и последующего сохранения: повторного скачивания для Save не требуется.
Generic library.import/replace обслуживают законченный preview.
Resource ID — однозначный JSON ключ `[username,remoteFilename]`, без пароля.

## Preview и загрузка

1. Worker отправляет preview request и запоминает намерение воспроизвести/сохранить.
2. Native пишет в отдельный staging-файл и публикует `soulseek-preview`.
3. Состояния queued_local/queued_remote/progress/completed/failed/timed_out/
   cancelled передаются с transfer ID, байтами, скоростью и позицией очереди.
4. Буфер для старта: минимум 384 KiB, цель 18 секунд по bitrate, fallback
   1.5 MB, ограничение размером файла. Некоторые контейнеры требуют больше данных.
5. После готовности Worker получает sourceToken и запускает общий GrowingFile.
6. Save ожидает completion, импортирует через ядро и принимает Track в очередь.
7. Replace ожидает completion и заменяет выбранный библиотечный Track по ID.

Отмена и disable отменяют download handles; эпоха native-адаптера исключает
поздний connect после отключения. Новая попытка имеет отдельный файл и canceller.
Терминальное состояние всегда освобождает ожидание EOF. Covers загружаются
отдельно и не задерживают вывод результатов поиска.

Native snapshot событий восстанавливает transfer progress после UI reload.
Сами результаты поиска можно запросить заново; resource identity активного
трека восстанавливается из audio session. Намерения, которые должны пережить
reload, сохраняются в plugin settings, а не в refs основной оболочки.

## Миграция и инварианты

Legacy enabled/username/password читаются из device_config. Enabled и username
переносятся в plugin registry, пароль — в credential provider; legacy secret
очищается только после успешной записи. Обычный импорт использует общие
`import_paths`, не зависит от наличия Soulseek feature. Старые `.soulseek-*`
каталоги остаются исключёнными из сканирования для совместимости старых данных.

Приёмка: отсутствие credentials, поиск/обложки, remote queue, preview/cancel,
продолжение после EOF при растущем файле, Save без повторной загрузки, замена,
reload, disable во время connect/download, повторное включение и очистка cache.
