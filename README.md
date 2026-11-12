<img src="docs/design/app-icon.png" width="96" align="right" alt="Иконка Мостика: висячий мост, трос которого рисует букву «М»">

# Мостик

Неофициальный клиент чатов Яндекс Телемоста для Linux, Windows и macOS.

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
- Файлы и картинки: через «+» или перетаскиванием.
- Обновления в реальном времени, уведомления, значок в трее, статусы и присутствие.
- Поиск по загруженным сообщениям (Ctrl+K).
- Светлая и тёмная темы, интерфейс на русском и английском.
- Созвоны в отдельном окне.

Поиска по всей истории на сервере пока нет.

| Светлая тема | Настройки |
|---|---|
| ![Мостик: светлая тема «Мох», панель действий над сообщением](docs/screenshots/light.png) | ![Мостик: настройки оформления, темы Мостика и классические](docs/screenshots/preferences.png) |

## Установка

Скачайте файл из [последнего релиза](https://github.com/tsarev-i/Mostik/releases/latest):

- Linux: `Mostik_*_amd64.AppImage`, перед первым запуском `chmod +x`.
- Windows: установщик `Mostik_*_x64-setup.exe`.
- macOS (только Apple Silicon, M1 и новее): образ `Mostik_*_aarch64.dmg`, перетащите Мостик в «Программы».
  Сборка не подписана сертификатом Apple, поэтому при первом запуске macOS его блокирует:
  разрешите запуск в «Системные настройки -> Конфиденциальность и безопасность -> Всё равно открыть»
  или выполните `xattr -dr com.apple.quarantine /Applications/Mostik.app`.

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
На Linux ещё WebKitGTK 4.1:

```bash
sudo apt install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev   # Debian/Ubuntu
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
включаются переменной `MOSTIK_DEVTOOLS=1`.

| Путь | Что внутри |
|---|---|
| `crates/mostik-core` | Модель (чаты, сообщения, люди), разбор разметки, демо-данные |
| `crates/mostik-yandex` | Протокол веб-клиента Телемоста: HTTP RPC и WebSocket Xiva |
| `app/ui` | Интерфейс на Leptos (wasm) |
| `app/src-tauri` | Десктоп-оболочка на Tauri 2: вход, хранение сессии, уведомления, трей |
| `docs/design.md` | Размеры, цвета и поведение интерфейса |

## Лицензия

[MIT](LICENSE).

---

**In English:** Mostik is an unofficial desktop client for Yandex Telemost chats on Linux, Windows and macOS,
written in Rust with Tauri 2 and Leptos. It is not affiliated with Yandex LLC.
