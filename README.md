<img src="docs/design/app-icon.png" width="96" align="right" alt="Иконка Мостика: висячий мост, трос которого рисует букву «М»">

# Мостик

Неофициальный клиент чатов Яндекс Телемоста для Linux, Windows и macOS.

[![Последняя версия](https://img.shields.io/github/v/release/tsarev-i/Mostik?label=%D0%B2%D0%B5%D1%80%D1%81%D0%B8%D1%8F)](https://github.com/tsarev-i/Mostik/releases/latest)
[![Лицензия MIT](https://img.shields.io/github/license/tsarev-i/Mostik?label=%D0%BB%D0%B8%D1%86%D0%B5%D0%BD%D0%B7%D0%B8%D1%8F)](LICENSE)

**Скачать:** [Linux (AppImage)](https://github.com/tsarev-i/Mostik/releases/latest/download/Mostik_amd64.AppImage) ·
[Windows](https://github.com/tsarev-i/Mostik/releases/latest/download/Mostik_x64-setup.exe) ·
[macOS (Apple Silicon)](https://github.com/tsarev-i/Mostik/releases/latest/download/Mostik_aarch64.dmg)

> [!WARNING]
> Мостик не связан с ООО «Яндекс» и не поддерживается им. Он работает через недокументированный
> протокол веб-клиента Телемоста, который может измениться в любой момент.
> «Яндекс» и «Телемост» являются товарными знаками их правообладателя.

![Мостик: канал #dev в тёмной теме «Сумерки», в сайдбаре разделы и «Чаты встреч», справа открыт тред](docs/screenshots/dark-thread.png)

## Возможности

- Каналы, группы, личные сообщения, треды, чаты встреч и каналы-рассылки.
- Правка, удаление, реакции, упоминания, отметка «важное», разметка и блоки кода.
- Ответ на сообщение и цитирование выделенного фрагмента, совместимые с официальным клиентом.
- Свои emoji в тексте и в реакциях, общие для всех в организации, кто пользуется Мостиком.
- Файлы и картинки в чате и в треде: через «+», перетаскиванием или вставкой из буфера обмена.
- Отметки о прочтении, при наведении на время своего сообщения.
- Обновления в реальном времени, уведомления, значок в трее, статусы и присутствие, «Не беспокоить» вместе с Телемостом.
- Поиск по загруженным сообщениям (Ctrl+K).
- Светлая и тёмная темы, интерфейс на русском и английском.
- Созвоны в отдельном окне.

Поиска по всей истории на сервере пока нет.

| Светлая тема | Настройки |
|---|---|
| ![Мостик: светлая тема «Мох», панель действий над сообщением](docs/screenshots/light.png) | ![Мостик: настройки оформления, темы Мостика и классические](docs/screenshots/preferences.png) |

## Установка

Ссылки ведут на файлы последней версии:

| Система | Файл |
|---|---|
| Linux x86-64 с glibc 2.35 и новее: Ubuntu 22.04+, Debian 12+, Fedora 36+, Arch Linux | [Mostik_amd64.AppImage](https://github.com/tsarev-i/Mostik/releases/latest/download/Mostik_amd64.AppImage) |
| Windows 10 и 11, x64 | [Mostik_x64-setup.exe](https://github.com/tsarev-i/Mostik/releases/latest/download/Mostik_x64-setup.exe) |
| macOS на Apple Silicon (M1 и новее) | [Mostik_aarch64.dmg](https://github.com/tsarev-i/Mostik/releases/latest/download/Mostik_aarch64.dmg) |

Прошлые версии лежат на [странице релизов](https://github.com/tsarev-i/Mostik/releases),
список изменений — в [CHANGELOG](CHANGELOG.md).

## Обновления

Мостик проверяет релизы на GitHub при запуске и раз в 4 часа. Когда выходит новая версия, в верхней
панели появляется кнопка «Обновить»; без вашего клика ничего не устанавливается. Обновления подписаны,
и Мостик проверяет подпись перед установкой. Автоматическую проверку можно выключить в настройках.


## Вход и данные

Вход через Яндекс ID, в том числе корпоративный SSO, на странице Яндекса в окне приложения.
Мостик хранит cookie сессии в системном хранилище паролей. На время созвона они попадают ещё
в профиль окна созвона: на Linux он лежит в памяти (`$XDG_RUNTIME_DIR`) и удаляется вместе
с созвоном, на Windows окно созвона приватное и ничего не пишет на диск. «Выйти» выходит
из Яндекс ID, как веб-версия Телемоста, и удаляет данные аккаунта с компьютера. Мостик обращается
к серверам Яндекса, где представляется в `User-Agent` как `Mostik/<версия>`, и к релизам
на GitHub, чтобы проверить обновления.

## Сборка из исходников

Написан на Rust (Tauri 2 и Leptos). Понадобятся Rust 1.94+, цель `wasm32-unknown-unknown`,
`wasm-bindgen-cli` той же версии, что `wasm-bindgen` в `Cargo.lock`, и `tauri-cli` 2.
На Linux ещё WebKitGTK 4.1, библиотека значка в трее и ALSA для звуков:

```bash
sudo apt install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libasound2-dev   # Debian/Ubuntu
sudo pacman -S --needed base-devel webkit2gtk-4.1 libayatana-appindicator librsvg alsa-lib rustup   # Arch Linux
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.130 --locked
cargo install tauri-cli --version 2.12.4 --locked
```

```bash
scripts/ui.sh serve                              # интерфейс в браузере на демо-данных: http://127.0.0.1:1420
scripts/ui.sh build && cargo run -p mostik-app   # приложение
cargo tauri build                                # AppImage на Linux, установщик на Windows
cargo tauri build --bundles app,dmg              # .app и DMG на macOS
cargo test --workspace                           # тесты
```

На Windows скрипты запускаются из Git Bash. Инструменты разработчика в отладочной сборке
включаются переменной `MOSTIK_DEVTOOLS=1`. Диагностика подключения (`probe*.json` в каталоге
данных приложения: структура ответов и шаблоны идентификаторов, без текстов, имён и значений
cookie) пишется в отладочной сборке, а в релизной — с переменной `MOSTIK_PROBE=1`.

| Путь | Что внутри |
|---|---|
| `crates/mostik-core` | Модель (чаты, сообщения, люди), разбор разметки, демо-данные |
| `crates/mostik-yandex` | Протокол веб-клиента Телемоста: HTTP RPC и WebSocket Xiva |
| `app/ui` | Интерфейс на Leptos (wasm) |
| `app/src-tauri` | Десктоп-оболочка на Tauri 2: вход, хранение сессии, уведомления, трей |
| `docs/design.md` | Размеры, цвета и поведение интерфейса |

## Ошибки и предложения

Пишите в [Issues](https://github.com/tsarev-i/Mostik/issues). К ошибке приложите версию Мостика
(«О программе» в верхней панели), систему и шаги, после которых она появляется.

## Лицензия

[MIT](LICENSE).

---

**In English:** Mostik is an unofficial desktop client for Yandex Telemost chats on Linux, Windows and macOS,
written in Rust with Tauri 2 and Leptos. It is not affiliated with Yandex LLC.
