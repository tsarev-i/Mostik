//! UI strings in Russian and English.
//!
//! The language is chosen in "Preferences → Language & region"; the default is the system language.
//! Changing the language re-renders the whole workspace, so
//! components read strings non-reactively via [`tr`].

use mostik_core::{NotifyLevel, SystemNote, UserId};
use serde::{Deserialize, Serialize};

use crate::settings::{SidebarFilter, SidebarSort};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Lang {
    #[default]
    Ru,
    En,
}

impl Lang {
    /// System language: Russian for `ru-*`, otherwise English.
    pub fn system() -> Self {
        let lang = web_sys::window()
            .and_then(|w| w.navigator().language())
            .unwrap_or_default();
        if lang.to_lowercase().starts_with("ru") {
            Lang::Ru
        } else {
            Lang::En
        }
    }

    /// Language code passed to the backend.
    pub fn code(self) -> &'static str {
        match self {
            Lang::Ru => "ru",
            Lang::En => "en",
        }
    }
}

/// All UI strings of one language.
pub struct Tr {
    pub lang: Lang,
    // Common
    pub app_name: &'static str,
    pub unofficial_client: &'static str,
    pub close: &'static str,
    pub cancel: &'static str,
    pub delete: &'static str,
    pub more: &'static str,
    pub drag_to_resize: &'static str,
    pub connecting: &'static str,
    pub finish_login: &'static str,
    pub signing_out: &'static str,
    pub offline: &'static str,
    pub you_short: &'static str,
    pub someone: &'static str,
    pub user: &'static str,
    /// A mentioned person whose name is not known yet.
    pub unknown_user: &'static str,
    pub bot: &'static str,
    pub notes: &'static str,
    // Login
    pub login_button: &'static str,
    pub login_note_password: &'static str,
    pub login_note_unofficial: &'static str,
    // Top bar and rail
    pub back: &'static str,
    pub forward: &'static str,
    pub history: &'static str,
    pub about: &'static str,
    pub search_chats: &'static str,
    pub search_messages: &'static str,
    pub nothing_found: &'static str,
    pub find_conversation: &'static str,
    pub clear_find: &'static str,
    pub no_conversations_found: &'static str,
    pub your_channels: &'static str,
    pub recommended: &'static str,
    pub search_channels_ph: &'static str,
    pub join_btn: &'static str,
    pub open_btn: &'static str,
    pub joined: &'static str,
    pub err_join: &'static str,
    pub vis_public_hint: &'static str,
    pub vis_public: &'static str,
    pub vis_private_hint: &'static str,
    pub vis_private: &'static str,
    pub visibility: &'static str,
    pub act_empty: &'static str,
    pub act_reactions: &'static str,
    pub act_threads: &'static str,
    pub act_mentions: &'static str,
    pub act_all: &'static str,
    pub rail_activity: &'static str,
    pub organizations: &'static str,
    pub switch_org: &'static str,
    pub browse_channels: &'static str,
    pub expand_all: &'static str,
    pub collapse_all: &'static str,
    pub expand_section: &'static str,
    pub collapse_section: &'static str,
    pub mark_all_read: &'static str,
    pub view_in_chat: &'static str,
    pub no_pins: &'static str,
    pub tab_pins: &'static str,
    pub forward_btn: &'static str,
    pub forward_comment_ph: &'static str,
    pub forward_to_ph: &'static str,
    pub err_forward: &'static str,
    pub forwarded_notice: &'static str,
    pub pinned_label: &'static str,
    pub unpin_message: &'static str,
    pub pin_message: &'static str,
    pub forward_message: &'static str,
    pub link_copied: &'static str,
    pub copy_link: &'static str,
    pub mark_unread: &'static str,
    pub read: &'static str,
    pub not_seen_yet: &'static str,
    pub not_read_yet: &'static str,
    pub read_by_all: &'static str,
    /// Heading of a chat's notification choices in its menu.
    pub notify_label: &'static str,
    pub notify_all: &'static str,
    pub notify_mentions: &'static str,
    pub notify_nothing: &'static str,
    pub unstar: &'static str,
    pub star: &'static str,
    pub mark_as_read: &'static str,
    pub err_pin: &'static str,
    pub notice_notify_all: &'static str,
    pub notice_notify_mentions: &'static str,
    pub notice_notify_nothing: &'static str,
    pub err_settings: &'static str,
    pub err_call: &'static str,
    pub call_opening: &'static str,
    pub call_in_progress: &'static str,
    pub call_leave: &'static str,
    pub call_go_to_chat: &'static str,
    pub call_busy: &'static str,
    pub call_started: &'static str,
    pub call_incoming: &'static str,
    pub call_missed: &'static str,
    pub call_declined: &'static str,
    pub call_no_answer: &'static str,
    pub call_direct: &'static str,
    pub call_done: &'static str,
    pub ring_calling: &'static str,
    pub ring_decline: &'static str,
    pub ring_accept: &'static str,
    pub call_join: &'static str,
    pub call_live_hint: &'static str,
    pub in_meeting: &'static str,
    pub no_history: &'static str,
    pub recent_chats: &'static str,
    pub searching: &'static str,
    pub search_people: &'static str,
    pub rail_home: &'static str,
    pub rail_dms: &'static str,
    pub profile_and_prefs: &'static str,
    // Sidebar
    pub sec_starred: &'static str,
    pub sec_channels: &'static str,
    pub sec_dms: &'static str,
    pub sec_meetings: &'static str,
    // Chat header
    pub members: &'static str,
    pub call: &'static str,
    pub call_hint: &'static str,
    pub tab_messages: &'static str,
    // Feed
    pub intro_notes_bold: &'static str,
    pub intro_notes_text: &'static str,
    pub intro_channel: &'static str,
    pub today: &'static str,
    pub yesterday: &'static str,
    /// "today" and "yesterday" inside a sentence: "today at 10:41".
    pub today_in_text: &'static str,
    pub yesterday_in_text: &'static str,
    pub edited: &'static str,
    pub important: &'static str,
    pub add_reaction: &'static str,
    pub search_emoji: &'static str,
    pub search_reactions: &'static str,
    pub frequently_used: &'static str,
    pub all_reactions: &'static str,
    pub no_emoji_found: &'static str,
    pub pick_emoji: &'static str,
    pub emoji_btn: &'static str,
    pub mention_all_hint: &'static str,
    pub mention_channel_hint: &'static str,
    pub emoji_groups: [&'static str; 9],
    pub open_thread: &'static str,
    pub reply_in_thread: &'static str,
    pub reaction: &'static str,
    pub mark_important: &'static str,
    pub unmark_important: &'static str,
    pub download_and_open: &'static str,
    pub copy_text: &'static str,
    pub reply: &'static str,
    pub quote_selection: &'static str,
    pub copy_selection: &'static str,
    pub cancel_reply: &'static str,
    pub go_to_original: &'static str,
    pub reply_deleted: &'static str,
    pub copy_name: &'static str,
    pub view_channel_details: &'static str,
    pub view_full_profile: &'static str,
    pub edit: &'static str,
    pub delete_message: &'static str,
    pub delete_message_warning: &'static str,
    // Composer
    pub reply_placeholder: &'static str,
    pub note_placeholder: &'static str,
    pub fmt_bold: &'static str,
    pub fmt_italic: &'static str,
    pub fmt_underline: &'static str,
    pub fmt_strike: &'static str,
    pub fmt_link: &'static str,
    pub fmt_code: &'static str,
    pub fmt_code_block: &'static str,
    pub link_add: &'static str,
    pub link_edit: &'static str,
    pub link_text: &'static str,
    pub link_address: &'static str,
    pub link_remove: &'static str,
    pub hint_shift_enter: &'static str,
    pub hint_ctrl_enter: &'static str,
    pub send: &'static str,
    pub thread: &'static str,
    // Avatar menu
    pub active: &'static str,
    pub preferences: &'static str,
    // About
    pub about_title: &'static str,
    pub update_button: &'static str,
    pub p_updates: &'static str,
    pub custom_emoji: &'static str,
    pub custom_emoji_hint: &'static str,
    pub add_emoji: &'static str,
    pub no_custom_emoji: &'static str,
    pub emoji_author: &'static str,
    pub emoji_taken: &'static str,
    pub emoji_rename: &'static str,
    pub emoji_delete_hint: &'static str,
    pub add_emoji_title: &'static str,
    pub emoji_upload_step: &'static str,
    pub emoji_name_step: &'static str,
    pub emoji_change_image: &'static str,
    pub emoji_name_ph: &'static str,
    pub emoji_choose_image: &'static str,
    pub emoji_image_hint: &'static str,
    pub emoji_name_hint: &'static str,
    pub emoji_adding: &'static str,
    pub emoji_offline: &'static str,
    pub unknown_custom_emoji: &'static str,
    pub p_auto_update: &'static str,
    pub p_auto_update_hint: &'static str,
    pub update_unsupported: &'static str,
    pub up_to_date: &'static str,
    pub check_updates: &'static str,
    pub checking_updates: &'static str,
    pub about_bold: &'static str,
    pub about_trademarks: &'static str,
    pub about_protocol: &'static str,
    // Errors
    pub err_history: &'static str,
    pub err_older: &'static str,
    pub err_thread: &'static str,
    pub err_delete: &'static str,
    pub err_reaction: &'static str,
    pub err_important: &'static str,
    pub err_download: &'static str,
    pub err_not_sent: &'static str,
    pub only_in_app_calls: &'static str,
    pub only_in_app_notifications: &'static str,
    pub saved_to: &'static str,
    // Sidebar filter and sorting
    pub manage_sidebar: &'static str,
    pub filter_and_sort: &'static str,
    pub filter_by: &'static str,
    pub f_active: &'static str,
    pub f_unreads: &'static str,
    pub f_mentions: &'static str,
    pub f_all: &'static str,
    pub sort_by: &'static str,
    pub s_az: &'static str,
    pub s_recency: &'static str,
    pub s_custom: &'static str,
    pub reset_order: &'static str,
    pub hide_chat: &'static str,
    pub create_section: &'static str,
    pub new_section_title: &'static str,
    pub new_section_item: &'static str,
    pub rename_section: &'static str,
    pub section_options: &'static str,
    pub manage_section: &'static str,
    pub sidebar_settings: &'static str,
    pub filter: &'static str,
    pub sort: &'static str,
    pub delete_section: &'static str,
    pub move_to_section: &'static str,
    pub section_name: &'static str,
    pub section_name_ph: &'static str,
    pub section_emoji: &'static str,
    pub remove_emoji: &'static str,
    pub hide_muted: &'static str,
    pub show_muted: &'static str,
    pub p_home: &'static str,
    pub p_active_hint: &'static str,
    pub p_muted: &'static str,
    // Preferences
    pub p_notifications: &'static str,
    pub p_appearance: &'static str,
    pub p_messages_media: &'static str,
    pub p_language_region: &'static str,
    pub p_mark_as_read: &'static str,
    pub p_advanced: &'static str,
    pub p_accessibility: &'static str,
    pub p_zoom: &'static str,
    pub p_zoom_hint: &'static str,
    pub p_notify_about: &'static str,
    pub p_notify_all: &'static str,
    pub p_notify_direct: &'static str,
    pub p_notify_nothing: &'static str,
    pub p_notify_preview: &'static str,
    pub p_notify_explain: &'static str,
    pub p_show_example: &'static str,
    pub p_color_mode: &'static str,
    pub p_mode_light: &'static str,
    pub p_mode_dark: &'static str,
    pub p_mode_system: &'static str,
    pub p_themes: &'static str,
    /// Theme group headings in Preferences → Appearance: Mostik's own themes and the classic ones.
    pub themes_own: &'static str,
    pub themes_classic: &'static str,
    pub p_inline_media: &'static str,
    pub p_inline_media_hint: &'static str,
    pub p_clock: &'static str,
    pub p_clock24: &'static str,
    pub p_language: &'static str,
    pub p_language_hint: &'static str,
    pub p_when_view: &'static str,
    pub p_read_on_open: &'static str,
    pub p_leave_unread: &'static str,
    pub p_read_receipts: &'static str,
    pub p_show_read_receipts: &'static str,
    pub p_enter_to: &'static str,
    pub p_enter_send: &'static str,
    pub p_enter_newline: &'static str,
    pub theme_names: [&'static str; 7],
    // Profile
    pub profile: &'static str,
    pub message_button: &'static str,
    pub contact_info: &'static str,
    pub email_address: &'static str,
    pub department: &'static str,
    pub organization: &'static str,
    pub nickname: &'static str,
    pub err_profile: &'static str,
    pub err_open_dm: &'static str,
    // Drafts and sent
    pub drafts_and_sent: &'static str,
    pub threads: &'static str,
    pub no_threads: &'static str,
    pub no_threads_hint: &'static str,
    pub drafts: &'static str,
    pub sent: &'static str,
    pub no_drafts: &'static str,
    pub no_drafts_hint: &'static str,
    pub no_sent: &'static str,
    pub delete_draft: &'static str,
    pub you_label: &'static str,
    pub pause_for: &'static str,
    pub pause_30m: &'static str,
    pub pause_1h: &'static str,
    pub pause_2h: &'static str,
    pub pause_tomorrow: &'static str,
    pub paused_until: &'static str,
    pub update_status: &'static str,
    pub set_away: &'static str,
    pub set_active: &'static str,
    pub resume_notifications: &'static str,
    pub away: &'static str,
    pub busy: &'static str,
    pub dnd: &'static str,
    pub offline_status: &'static str,
    pub status_placeholder: &'static str,
    pub clear_after: &'static str,
    pub clear_never: &'static str,
    pub clear_30m: &'static str,
    pub clear_1h: &'static str,
    pub clear_4h: &'static str,
    pub clear_today: &'static str,
    pub clear_status: &'static str,
    pub err_status: &'static str,
    pub for_your_org: &'static str,
    pub status_presets: [(&'static str, &'static str); 5],
    pub p_sound: &'static str,
    pub attach: &'static str,
    pub only_in_app_files: &'static str,
    pub remove_file: &'static str,
    pub uploading: &'static str,
    pub new_message: &'static str,
    pub new_message_to: &'static str,
    pub write_to_colleague: &'static str,
    pub edit_profile: &'static str,
    pub local_time: &'static str,
    pub add_channels: &'static str,
    pub create_channel: &'static str,
    pub channel_name: &'static str,
    pub channel_name_ph: &'static str,
    pub channel_desc: &'static str,
    pub channel_desc_ph: &'static str,
    pub next: &'static str,
    pub create: &'static str,
    pub add_people: &'static str,
    pub add_people_to: &'static str,
    pub find_members: &'static str,
    pub add_btn: &'static str,
    pub search_people_ph: &'static str,
    pub skip_for_now: &'static str,
    pub about_channel: &'static str,
    pub members_tab: &'static str,
    pub channel_description: &'static str,
    pub no_description: &'static str,
    pub save: &'static str,
    pub save_changes: &'static str,
    pub leave_channel: &'static str,
    pub leave_confirm: &'static str,
    pub admin: &'static str,
    pub remove_from_channel: &'static str,
    pub err_create_channel: &'static str,
    pub err_members: &'static str,
    pub err_update_chat: &'static str,
    pub err_leave: &'static str,
    pub err_search: &'static str,
    pub only_admins_edit: &'static str,
    pub in_app_only_groups: &'static str,
}

/// Russian strings.
pub static RU: Tr = Tr {
    lang: Lang::Ru,
    app_name: "Мостик",
    unofficial_client: "Неофициальный клиент Яндекс Телемоста",
    close: "Закрыть",
    cancel: "Отмена",
    delete: "Удалить",
    more: "Ещё",
    drag_to_resize: "Потяните, чтобы изменить ширину",
    connecting: "Подключаемся к Телемосту…",
    finish_login: "Завершите вход в окне Яндекс ID…",
    signing_out: "Выходим из аккаунта…",
    offline: "Нет соединения с Телемостом. Переподключаемся…",
    you_short: "вы",
    someone: "Кто-то",
    user: "Пользователь",
    unknown_user: "Неизвестный пользователь",
    bot: "БОТ",
    notes: "Заметки",
    login_button: "Войти через Яндекс ID",
    login_note_password: "Откроется страница Яндекс ID. Логин и пароль вы вводите только там — Мостик их не видит. После входа сессия хранится в системном хранилище паролей.",
    login_note_unofficial: "Мостик не создан и не одобрен ООО «Яндекс» и работает через недокументированный протокол веб-клиента.",
    back: "Назад",
    forward: "Вперёд",
    history: "История",
    about: "О программе",
    search_chats: "Чаты",
    search_messages: "Сообщения",
    nothing_found: "Ничего не найдено",
    find_conversation: "Найти беседу…",
    clear_find: "Очистить",
    no_conversations_found: "Таких бесед в списке нет",
    your_channels: "Ваши каналы",
    recommended: "Рекомендуемые",
    search_channels_ph: "Поиск каналов",
    join_btn: "Присоединиться",
    open_btn: "Открыть",
    joined: "Вы участник",
    err_join: "Не удалось присоединиться",
    vis_public_hint: "его может найти и открыть любой в организации",
    vis_public: "Открытый",
    vis_private_hint: "только для приглашённых",
    vis_private: "Закрытый",
    visibility: "Видимость",
    act_empty: "Здесь появятся упоминания, ответы и реакции",
    act_reactions: "Реакции",
    act_threads: "Треды",
    act_mentions: "Упоминания",
    act_all: "Все",
    rail_activity: "События",
    organizations: "Организации",
    switch_org: "Переключить организацию",
    browse_channels: "Найти каналы",
    expand_all: "Развернуть все разделы",
    collapse_all: "Свернуть все разделы",
    expand_section: "Развернуть раздел",
    collapse_section: "Свернуть раздел",
    mark_all_read: "Отметить все прочитанными",
    view_in_chat: "Показать в чате",
    no_pins: "В этом чате ничего не закреплено",
    tab_pins: "Закреплённое",
    forward_btn: "Переслать",
    forward_comment_ph: "Добавьте комментарий, если хотите",
    forward_to_ph: "Куда: канал или человек",
    err_forward: "Не удалось переслать",
    forwarded_notice: "Сообщение переслано",
    pinned_label: "Закреплено в чате",
    unpin_message: "Открепить",
    pin_message: "Закрепить в чате",
    forward_message: "Переслать сообщение",
    link_copied: "Ссылка скопирована",
    copy_link: "Копировать ссылку",
    mark_unread: "Пометить непрочитанным",
    read: "Прочитано",
    not_seen_yet: "Ещё не прочитано",
    not_read_yet: "Ещё никто не прочитал",
    read_by_all: "Прочитали все",
    notify_label: "Уведомления",
    notify_all: "Все сообщения",
    notify_mentions: "Только упоминания",
    notify_nothing: "Ничего",
    unstar: "Убрать из избранного",
    star: "Добавить в избранное",
    mark_as_read: "Отметить прочитанным",
    err_pin: "Не удалось закрепить сообщение",
    notice_notify_all: "Чат уведомляет обо всех сообщениях",
    notice_notify_mentions: "Чат уведомляет только об упоминаниях",
    notice_notify_nothing: "Уведомления из чата отключены",
    err_settings: "Не удалось сохранить",
    err_call: "Не удалось начать звонок",
    call_opening: "Открываем созвон…",
    call_in_progress: "Созвон",
    call_leave: "Выйти",
    call_go_to_chat: "Перейти в чат созвона",
    call_busy: "Сначала выйдите из текущего созвона",
    call_started: "Созвон начался",
    call_incoming: "Входящий звонок",
    call_missed: "Пропущенный звонок",
    call_declined: "Звонок отклонён",
    call_no_answer: "Нет ответа",
    call_direct: "Звонок",
    call_done: "Созвон",
    ring_calling: "звонит вам…",
    ring_decline: "Отклонить",
    ring_accept: "Принять",
    call_join: "Присоединиться",
    call_live_hint: "Сейчас идёт созвон",
    in_meeting: "На встрече",
    no_history: "Здесь появятся недавно открытые чаты",
    recent_chats: "Недавние",
    searching: "Ищем…",
    search_people: "Люди",
    rail_home: "Главная",
    rail_dms: "Личные",
    profile_and_prefs: "Профиль и настройки",
    sec_starred: "Избранное",
    sec_channels: "Каналы",
    sec_dms: "Личные сообщения",
    sec_meetings: "Чаты встреч",
    members: "Участники",
    call: "Созвон",
    call_hint: "Созвон в Телемосте",
    tab_messages: "Сообщения",
    intro_notes_bold: "Это ваше личное пространство. ",
    intro_notes_text: "Набрасывайте черновики, ведите список дел, храните ссылки и файлы под рукой. Можно и поговорить с собой, но учтите: отвечать придётся тоже вам.",
    intro_channel: "Это самое начало канала.",
    today: "Сегодня",
    yesterday: "Вчера",
    today_in_text: "сегодня",
    yesterday_in_text: "вчера",
    edited: "(изменено)",
    important: "Важное",
    add_reaction: "Добавить реакцию",
    search_emoji: "Поиск эмодзи",
    search_reactions: "Поиск реакций",
    frequently_used: "Часто используемые",
    all_reactions: "Реакции Телемоста",
    no_emoji_found: "Ничего не найдено",
    pick_emoji: "Выберите эмодзи",
    emoji_btn: "Эмодзи",
    mention_all_hint: "Уведомить всех в этом чате",
    mention_channel_hint: "Участники канала",
    emoji_groups: [
        "Смайлики и эмоции",
        "Люди и тело",
        "Животные и природа",
        "Еда и напитки",
        "Путешествия и места",
        "Занятия",
        "Предметы",
        "Символы",
        "Флаги",
    ],
    open_thread: "Открыть тред",
    reply_in_thread: "Ответить в треде",
    reaction: "Реакция",
    mark_important: "Отметить как важное",
    unmark_important: "Снять отметку «Важное»",
    download_and_open: "Скачать и открыть",
    copy_text: "Скопировать текст",
    reply: "Ответить",
    quote_selection: "Цитировать выделенное",
    copy_selection: "Скопировать выделенное",
    cancel_reply: "Отменить ответ",
    go_to_original: "Перейти к сообщению",
    reply_deleted: "Сообщение удалено",
    copy_name: "Копировать название",
    view_channel_details: "Открыть сведения о канале",
    view_full_profile: "Открыть полный профиль",
    edit: "Изменить",
    delete_message: "Удалить сообщение",
    delete_message_warning: "Сообщение удалится у всех участников чата. Отменить это нельзя.",
    reply_placeholder: "Ответить…",
    note_placeholder: "Заметка для себя",
    fmt_bold: "Жирный",
    fmt_italic: "Курсив",
    fmt_underline: "Подчёркнутый",
    fmt_strike: "Зачёркнутый",
    fmt_link: "Ссылка",
    fmt_code: "Код",
    fmt_code_block: "Блок кода",
    link_add: "Добавить ссылку",
    link_edit: "Изменить ссылку",
    link_text: "Текст",
    link_address: "Ссылка",
    link_remove: "Убрать ссылку",
    hint_shift_enter: "Shift+Enter — новая строка",
    hint_ctrl_enter: "Ctrl+Enter — отправить",
    send: "Отправить",
    thread: "Тред",
    active: "В сети",
    preferences: "Настройки",
    about_title: "О Мостике",
    update_button: "Обновить",
    p_updates: "Обновления",
    custom_emoji: "Кастомные emoji",
    custom_emoji_hint: "Свои emoji видят все в организации, кто пользуется Mostik: в тексте и в реакциях. В официальном Телемосте вместо них будет текст :имя:, а реакция окажется пустой.",
    add_emoji: "Добавить emoji",
    no_custom_emoji: "Пока нет ни одного emoji. Добавить его можно кнопкой выше или кнопкой «Добавить emoji» внизу окна выбора emoji. Первое добавленное создаст в организации служебный чат mostik-service: Mostik прячет его и хранит там emoji.",
    emoji_author: "Автор:",
    emoji_taken: "Имя уже занято более ранним emoji — переименуйте.",
    emoji_rename: "Переименовать",
    emoji_delete_hint: "Правый клик — удалить ваш emoji",
    add_emoji_title: "Добавить свой emoji",
    emoji_upload_step: "1. Загрузите картинку",
    emoji_name_step: "2. Придумайте имя",
    emoji_change_image: "Заменить картинку",
    emoji_name_ph: "имя",
    emoji_choose_image: "Выбрать картинку",
    emoji_image_hint: "Лучше всего подходят квадратные картинки с прозрачным фоном. Большую картинку Mostik уменьшит до 128 px; GIF — не больше 128 КБ.",
    emoji_name_hint: "Латинские строчные буквы, цифры, «_» и «-». Это же имя набирают в сообщении, чтобы вставить emoji.",
    emoji_adding: "Добавляю…",
    emoji_offline: "Кастомные emoji работают только в приложении.",
    unknown_custom_emoji: "Удалённый emoji",
    p_auto_update: "Проверять обновления автоматически",
    p_auto_update_hint: "Мостик проверяет релизы на GitHub при запуске и раз в 4 часа и показывает кнопку «Обновить» в верхней панели. Без вашего клика ничего не устанавливается.",
    update_unsupported: "Обновлять себя умеют только AppImage, версия из установщика для Windows и приложение для macOS.",
    up_to_date: "У вас последняя версия.",
    check_updates: "Проверить обновления",
    checking_updates: "Проверяем…",
    about_bold: "Мостик — неофициальный клиент Яндекс Телемоста.",
    about_trademarks: "Приложение не создано, не поддерживается и не одобрено ООО «Яндекс». «Яндекс» и «Телемост» — товарные знаки их правообладателя.",
    about_protocol: "Мостик работает с вашей учётной записью через тот же протокол, что и веб-клиент Телемоста. Пароль вы вводите только на странице Яндекс ID, приложение его не видит.",
    err_history: "Не удалось загрузить историю",
    err_older: "Не удалось загрузить старые сообщения",
    err_thread: "Не удалось загрузить тред",
    err_delete: "Не удалось удалить сообщение",
    err_reaction: "Не удалось изменить реакцию",
    err_important: "Не удалось отметить сообщение",
    err_download: "Не удалось скачать файл",
    err_not_sent: "Не отправлено",
    only_in_app_calls: "Созвоны работают только в приложении",
    only_in_app_notifications: "Уведомления работают только в приложении",
    saved_to: "Сохранено",
    // Sidebar filter and sorting
    manage_sidebar: "Управление боковой панелью",
    filter_and_sort: "Фильтр и сортировка",
    filter_by: "Фильтровать беседы:",
    f_active: "Только активные",
    f_unreads: "Непрочитанные",
    f_mentions: "Упоминания",
    f_all: "Все",
    sort_by: "Сортировать беседы:",
    s_az: "А–Я",
    s_recency: "По активности",
    s_custom: "Свой порядок",
    reset_order: "Сбросить свой порядок",
    hide_chat: "Скрыть из списка",
    create_section: "Создать раздел",
    new_section_title: "Новый раздел",
    new_section_item: "Новый раздел…",
    rename_section: "Переименовать раздел",
    section_options: "Параметры раздела",
    manage_section: "Раздел",
    sidebar_settings: "Все разделы",
    filter: "Фильтр",
    sort: "Сортировка",
    delete_section: "Удалить раздел",
    move_to_section: "Переместить в раздел",
    section_name: "Название",
    section_name_ph: "Например, «Проекты»",
    section_emoji: "Выбрать эмодзи",
    remove_emoji: "Убрать эмодзи",
    hide_muted: "Скрыть беседы без звука",
    show_muted: "Показать беседы без звука",
    p_home: "Главная",
    p_active_hint: "Скрывает беседы без новых сообщений за последние 30 дней.",
    p_muted: "Беседы без звука",
    p_notifications: "Уведомления",
    p_appearance: "Оформление",
    p_messages_media: "Сообщения и медиа",
    p_language_region: "Язык и регион",
    p_mark_as_read: "Отметка о прочтении",
    p_advanced: "Дополнительно",
    p_accessibility: "Специальные возможности",
    p_zoom: "Масштаб",
    p_zoom_hint: "Ctrl+= — увеличить, Ctrl+- — уменьшить, Ctrl+0 — исходный размер.",
    p_notify_about: "Уведомлять меня о…",
    p_notify_all: "Всех новых сообщениях",
    p_notify_direct: "Личных сообщениях и упоминаниях",
    p_notify_nothing: "Ни о чём",
    p_notify_preview: "Показывать текст сообщения в уведомлении",
    p_notify_explain: "Уведомления приходят, когда окно Мостика не в фокусе. Чат без звука уведомляет только об упоминаниях или не уведомляет совсем — это выбирается в его меню.",
    p_show_example: "Показать пример",
    p_color_mode: "Цветовой режим",
    p_mode_light: "Светлый",
    p_mode_dark: "Тёмный",
    p_mode_system: "Как в системе",
    p_themes: "Темы",
    themes_own: "Мостик",
    themes_classic: "Классические",
    p_inline_media: "Показывать картинки в ленте",
    p_inline_media_hint: "Если выключено, вместо картинки — ссылка с её названием.",
    p_clock: "Время",
    p_clock24: "24-часовой формат",
    p_language: "Язык",
    p_language_hint: "Язык интерфейса Мостика. Переписка не переводится.",
    p_when_view: "Когда я открываю чат",
    p_read_on_open: "Отмечать его прочитанным",
    p_leave_unread: "Оставлять непрочитанные сообщения непрочитанными",
    p_read_receipts: "Отметки о прочтении",
    p_show_read_receipts: "Показывать отметки о прочтении",
    p_enter_to: "При наборе сообщения Enter…",
    p_enter_send: "Отправляет сообщение",
    p_enter_newline: "Начинает новую строку (отправка — Ctrl+Enter)",
    theme_names: ["Сумерки", "Мох", "Вечер", "Полночь", "Лес", "Океан", "Закат"],
    profile: "Профиль",
    message_button: "Сообщение",
    contact_info: "Контактная информация",
    email_address: "Электронная почта",
    department: "Отдел",
    organization: "Организация",
    nickname: "Логин",
    err_profile: "Не удалось загрузить профиль",
    err_open_dm: "Не удалось открыть личный чат",
    drafts_and_sent: "Черновики и отправленные",
    threads: "Треды",
    no_threads: "Пока нет тредов",
    no_threads_hint: "Здесь появятся треды, которые вы начали, в которых отвечали или где вас упомянули.",
    drafts: "Черновики",
    sent: "Отправленные",
    no_drafts: "Черновиков нет",
    no_drafts_hint: "Недописанное сообщение сохраняется само и появляется здесь.",
    no_sent: "Вы ещё ничего не отправили в загруженных чатах",
    delete_draft: "Удалить черновик",
    you_label: "Вы",
    pause_for: "Приостановить уведомления",
    pause_30m: "На 30 минут",
    pause_1h: "На 1 час",
    pause_2h: "На 2 часа",
    pause_tomorrow: "До завтра",
    paused_until: "Уведомления приостановлены до",
    update_status: "Обновить статус",
    set_away: "Отметить: нет на месте",
    set_active: "Отметить: в сети",
    resume_notifications: "Включить уведомления",
    away: "Нет на месте",
    busy: "Занят",
    dnd: "Не беспокоить",
    offline_status: "Не в сети",
    status_placeholder: "Какой у вас статус?",
    clear_after: "Очистить через",
    clear_never: "Не очищать",
    clear_30m: "30 минут",
    clear_1h: "1 час",
    clear_4h: "4 часа",
    clear_today: "Сегодня",
    clear_status: "Очистить статус",
    err_status: "Не удалось изменить статус",
    for_your_org: "Предложения",
    status_presets: [
        ("🗓️", "На встрече"),
        ("🚌", "В дороге"),
        ("🤒", "Болею"),
        ("🌴", "В отпуске"),
        ("🏡", "Работаю из дома"),
    ],
    p_sound: "Звук уведомления",
    attach: "Прикрепить файл",
    only_in_app_files: "Отправка файлов работает только в приложении",
    remove_file: "Убрать файл",
    uploading: "Загружаем…",
    new_message: "Новое сообщение",
    new_message_to: "Кому:",
    write_to_colleague: "Написать коллеге",
    edit_profile: "Изменить",
    local_time: "местное время",
    add_channels: "Добавить каналы",
    create_channel: "Создать канал",
    channel_name: "Название",
    channel_name_ph: "например, план-релиза",
    channel_desc: "Описание (необязательно)",
    channel_desc_ph: "О чём этот канал?",
    next: "Далее",
    create: "Создать",
    add_people: "Добавить людей",
    add_people_to: "Добавить людей в",
    find_members: "Найти участников",
    add_btn: "Добавить",
    search_people_ph: "Введите имя или почту",
    skip_for_now: "Пропустить",
    about_channel: "О канале",
    members_tab: "Участники",
    channel_description: "Описание",
    no_description: "Описания нет",
    save: "Сохранить",
    save_changes: "Сохранить изменения",
    leave_channel: "Покинуть канал",
    leave_confirm: "Вы перестанете получать сообщения из этого канала. Вернуться можно, только если вас снова пригласят.",
    admin: "Админ",
    remove_from_channel: "Удалить из канала",
    err_create_channel: "Не удалось создать канал",
    err_members: "Не удалось изменить участников",
    err_update_chat: "Не удалось сохранить изменения",
    err_leave: "Не удалось покинуть канал",
    err_search: "Не удалось найти людей",
    only_admins_edit: "Менять название и участников могут только администраторы.",
    in_app_only_groups: "Управление каналами работает только в приложении",
};

/// English strings.
pub static EN: Tr = Tr {
    lang: Lang::En,
    app_name: "Mostik",
    unofficial_client: "Unofficial Yandex Telemost client",
    close: "Close",
    cancel: "Cancel",
    delete: "Delete",
    more: "More",
    drag_to_resize: "Drag to resize",
    connecting: "Connecting to Telemost…",
    finish_login: "Finish signing in in the Yandex ID window…",
    signing_out: "Signing out…",
    offline: "No connection to Telemost. Reconnecting…",
    you_short: "you",
    someone: "Someone",
    user: "User",
    unknown_user: "Unknown user",
    bot: "APP",
    notes: "Notes",
    login_button: "Sign in with Yandex ID",
    login_note_password: "A Yandex ID page will open. You enter your login and password only there — Mostik never sees them. After you sign in, the session is kept in the system password store.",
    login_note_unofficial: "Mostik is not made or endorsed by Yandex LLC and uses the undocumented protocol of the web client.",
    back: "Back",
    forward: "Forward",
    history: "History",
    about: "About",
    search_chats: "Chats",
    search_messages: "Messages",
    nothing_found: "No results",
    find_conversation: "Find a conversation…",
    clear_find: "Clear",
    no_conversations_found: "No conversations match",
    your_channels: "Your channels",
    recommended: "Recommended",
    search_channels_ph: "Search for channels",
    join_btn: "Join",
    open_btn: "View",
    joined: "Joined",
    err_join: "Couldn’t join the channel",
    vis_public_hint: "anyone in your organization can find and join",
    vis_public: "Public",
    vis_private_hint: "only people who are invited",
    vis_private: "Private",
    visibility: "Visibility",
    act_empty: "Mentions, replies and reactions will show up here",
    act_reactions: "Reactions",
    act_threads: "Threads",
    act_mentions: "Mentions",
    act_all: "All",
    rail_activity: "Activity",
    organizations: "Organizations",
    switch_org: "Switch organization",
    browse_channels: "Browse channels",
    expand_all: "Expand all sections",
    collapse_all: "Collapse all sections",
    expand_section: "Expand section",
    collapse_section: "Collapse section",
    mark_all_read: "Mark all as read",
    view_in_chat: "View in conversation",
    no_pins: "Nothing is pinned in this conversation",
    tab_pins: "Pins",
    forward_btn: "Forward",
    forward_comment_ph: "Add a message, if you’d like",
    forward_to_ph: "Add to a channel or DM",
    err_forward: "Couldn’t forward the message",
    forwarded_notice: "Message forwarded",
    pinned_label: "Pinned to this conversation",
    unpin_message: "Un-pin from this conversation",
    pin_message: "Pin to this conversation",
    forward_message: "Forward message",
    link_copied: "Link copied",
    copy_link: "Copy link",
    mark_unread: "Mark unread",
    read: "Read",
    not_seen_yet: "Not read yet",
    not_read_yet: "Not read yet",
    read_by_all: "Read by everyone",
    notify_label: "Notifications",
    notify_all: "All messages",
    notify_mentions: "Mentions only",
    notify_nothing: "Nothing",
    unstar: "Remove from Starred",
    star: "Star",
    mark_as_read: "Mark as read",
    err_pin: "Couldn’t pin the message",
    notice_notify_all: "This conversation notifies about all messages",
    notice_notify_mentions: "This conversation notifies about mentions only",
    notice_notify_nothing: "Notifications for this conversation are off",
    err_settings: "Couldn’t save the change",
    err_call: "Couldn’t start the call",
    call_opening: "Opening the call…",
    call_in_progress: "Call",
    call_leave: "Leave",
    call_go_to_chat: "Go to the call’s chat",
    call_busy: "Leave the current call first",
    call_started: "A call started",
    call_incoming: "Incoming call",
    call_missed: "Missed call",
    call_declined: "Call declined",
    call_no_answer: "No answer",
    call_direct: "Call",
    call_done: "Call",
    ring_calling: "is calling you…",
    ring_decline: "Decline",
    ring_accept: "Accept",
    call_join: "Join",
    call_live_hint: "A call is going on",
    in_meeting: "In a meeting",
    no_history: "Recently viewed conversations will show up here",
    recent_chats: "Recent",
    searching: "Searching…",
    search_people: "People",
    rail_home: "Home",
    rail_dms: "DMs",
    profile_and_prefs: "Profile and preferences",
    sec_starred: "Starred",
    sec_channels: "Channels",
    sec_dms: "Direct messages",
    sec_meetings: "Meeting chats",
    members: "Members",
    call: "Call",
    call_hint: "Telemost call",
    tab_messages: "Messages",
    intro_notes_bold: "This is your space. ",
    intro_notes_text: "Draft messages, list your to-dos, or keep links and files handy. You can also talk to yourself here, but please bear in mind you'll have to supply both sides of the conversation.",
    intro_channel: "This is the very beginning of the channel.",
    today: "Today",
    yesterday: "Yesterday",
    today_in_text: "today",
    yesterday_in_text: "yesterday",
    edited: "(edited)",
    important: "Important",
    add_reaction: "Add reaction",
    search_emoji: "Search all emoji",
    search_reactions: "Search reactions",
    frequently_used: "Frequently used",
    all_reactions: "Telemost reactions",
    no_emoji_found: "No emoji found",
    pick_emoji: "Pick an emoji",
    emoji_btn: "Emoji",
    mention_all_hint: "Notify everyone in this chat",
    mention_channel_hint: "Channel members",
    emoji_groups: [
        "Smileys & Emotion",
        "People & Body",
        "Animals & Nature",
        "Food & Drink",
        "Travel & Places",
        "Activities",
        "Objects",
        "Symbols",
        "Flags",
    ],
    open_thread: "View thread",
    reply_in_thread: "Reply in thread",
    reaction: "Reaction",
    mark_important: "Mark as important",
    unmark_important: "Remove “Important”",
    download_and_open: "Download and open",
    copy_text: "Copy text",
    reply: "Reply",
    quote_selection: "Quote selection",
    copy_selection: "Copy selection",
    cancel_reply: "Cancel reply",
    go_to_original: "Go to message",
    reply_deleted: "Message deleted",
    copy_name: "Copy name",
    view_channel_details: "View channel details",
    view_full_profile: "View full profile",
    edit: "Edit message",
    delete_message: "Delete message",
    delete_message_warning: "The message will be deleted for everyone in the chat. This can't be undone.",
    reply_placeholder: "Reply…",
    note_placeholder: "Jot something down",
    fmt_bold: "Bold",
    fmt_italic: "Italic",
    fmt_underline: "Underline",
    fmt_strike: "Strikethrough",
    fmt_link: "Link",
    fmt_code: "Code",
    fmt_code_block: "Code block",
    link_add: "Add link",
    link_edit: "Edit link",
    link_text: "Text",
    link_address: "Link",
    link_remove: "Remove link",
    hint_shift_enter: "Shift+Enter to add a new line",
    hint_ctrl_enter: "Ctrl+Enter to send",
    send: "Send now",
    thread: "Thread",
    active: "Active",
    preferences: "Preferences",
    about_title: "About Mostik",
    update_button: "Update",
    p_updates: "Updates",
    custom_emoji: "Custom emoji",
    custom_emoji_hint: "Everyone in the organization who uses Mostik sees these emoji, in texts and reactions. The official Telemost shows the text :name: instead and an empty reaction.",
    add_emoji: "Add emoji",
    no_custom_emoji: "No emoji yet. Add one with the button above or with “Add emoji” at the bottom of the emoji picker. The first one added creates the service chat mostik-service in the organization: Mostik hides it and keeps the emoji there.",
    emoji_author: "Added by",
    emoji_taken: "An earlier emoji has this name: rename it.",
    emoji_rename: "Rename",
    emoji_delete_hint: "Right-click to delete your emoji",
    add_emoji_title: "Add custom emoji",
    emoji_upload_step: "1. Upload an image",
    emoji_name_step: "2. Give it a name",
    emoji_change_image: "Choose another image",
    emoji_name_ph: "name",
    emoji_choose_image: "Choose image",
    emoji_image_hint: "Square images with a transparent background work best. Mostik scales a large image down to 128 px; a GIF must be 128 KB or less.",
    emoji_name_hint: "Lowercase Latin letters, digits, “_” and “-”. This is also what you type to add the emoji to a message.",
    emoji_adding: "Adding…",
    emoji_offline: "Custom emoji work only in the app.",
    unknown_custom_emoji: "Deleted emoji",
    p_auto_update: "Check for updates automatically",
    p_auto_update_hint: "Mostik checks GitHub releases at startup and every 4 hours and shows an Update button in the top bar. Nothing is installed until you click it.",
    update_unsupported: "Only the AppImage, the version installed with the Windows setup and the macOS app can update themselves.",
    up_to_date: "You are on the latest version.",
    check_updates: "Check for updates",
    checking_updates: "Checking…",
    about_bold: "Mostik is an unofficial Yandex Telemost client.",
    about_trademarks: "The app is not made, supported or endorsed by Yandex LLC. “Yandex” and “Telemost” are trademarks of their owner.",
    about_protocol: "Mostik works with your account through the same protocol as the Telemost web client. You enter your password only on the Yandex ID page; the app never sees it.",
    err_history: "Couldn't load history",
    err_older: "Couldn't load older messages",
    err_thread: "Couldn't load the thread",
    err_delete: "Couldn't delete the message",
    err_reaction: "Couldn't change the reaction",
    err_important: "Couldn't mark the message",
    err_download: "Couldn't download the file",
    err_not_sent: "Not sent",
    only_in_app_calls: "Calls work only in the desktop app",
    only_in_app_notifications: "Notifications work only in the desktop app",
    saved_to: "Saved",
    // Sidebar filter and sorting
    manage_sidebar: "Manage my sidebar",
    filter_and_sort: "Filter and sort",
    filter_by: "Filter conversations by:",
    f_active: "Active only",
    f_unreads: "Unreads",
    f_mentions: "Mentions",
    f_all: "All",
    sort_by: "Sort conversations by:",
    s_az: "A–Z",
    s_recency: "Recency",
    s_custom: "Custom",
    reset_order: "Reset custom order",
    hide_chat: "Hide from sidebar",
    create_section: "Create section",
    new_section_title: "New section",
    new_section_item: "New section…",
    rename_section: "Rename section",
    section_options: "Section options",
    manage_section: "Manage section",
    sidebar_settings: "All sections",
    filter: "Filter",
    sort: "Sort",
    delete_section: "Delete section",
    move_to_section: "Move to section",
    section_name: "Name",
    section_name_ph: "For example, “Projects”",
    section_emoji: "Choose an emoji",
    remove_emoji: "Remove emoji",
    hide_muted: "Hide muted conversations",
    show_muted: "Show muted conversations",
    p_home: "Home",
    p_active_hint: "Hides conversations with no new activity in the past 30 days.",
    p_muted: "Muted conversations",
    p_notifications: "Notifications",
    p_appearance: "Appearance",
    p_messages_media: "Messages & media",
    p_language_region: "Language & region",
    p_mark_as_read: "Mark as read",
    p_advanced: "Advanced",
    p_accessibility: "Accessibility",
    p_zoom: "Zoom",
    p_zoom_hint: "Use Ctrl+= to zoom in, Ctrl+- to zoom out and Ctrl+0 to reset.",
    p_notify_about: "Notify me about…",
    p_notify_all: "All new messages",
    p_notify_direct: "Direct messages and mentions",
    p_notify_nothing: "Nothing",
    p_notify_preview: "Include a preview of the message in each notification",
    p_notify_explain: "Notifications arrive when the Mostik window isn't focused. A muted chat notifies about mentions only or about nothing, as set in its menu.",
    p_show_example: "Show me an example",
    p_color_mode: "Color mode",
    p_mode_light: "Light",
    p_mode_dark: "Dark",
    p_mode_system: "Sync with OS",
    p_themes: "Themes",
    themes_own: "Mostik",
    themes_classic: "Classic",
    p_inline_media: "Show images inline",
    p_inline_media_hint: "When off, an image is shown as a link with its name.",
    p_clock: "Clock",
    p_clock24: "Show times with 24-hour clock",
    p_language: "Language",
    p_language_hint: "The language of the Mostik interface. Messages are not translated.",
    p_when_view: "When I view a chat",
    p_read_on_open: "Mark it as read",
    p_leave_unread: "Leave unseen messages unread",
    p_read_receipts: "Read receipts",
    p_show_read_receipts: "Show read receipts",
    p_enter_to: "When writing a message, press Enter to…",
    p_enter_send: "Send the message",
    p_enter_newline: "Start a new line (use Ctrl+Enter to send)",
    theme_names: ["Dusk", "Moss", "Twilight", "Midnight", "Forest", "Ocean", "Sunset"],
    profile: "Profile",
    message_button: "Message",
    contact_info: "Contact information",
    email_address: "Email address",
    department: "Department",
    organization: "Organization",
    nickname: "Username",
    err_profile: "Couldn't load the profile",
    err_open_dm: "Couldn't open the direct message",
    drafts_and_sent: "Drafts & sent",
    threads: "Threads",
    no_threads: "No threads yet",
    no_threads_hint: "Threads you started, replied in or were mentioned in show up here.",
    drafts: "Drafts",
    sent: "Sent",
    no_drafts: "No drafts",
    no_drafts_hint: "Unfinished messages are saved automatically and show up here.",
    no_sent: "You haven't sent anything in the loaded chats yet",
    delete_draft: "Delete draft",
    you_label: "You",
    pause_for: "Pause notifications",
    pause_30m: "For 30 minutes",
    pause_1h: "For 1 hour",
    pause_2h: "For 2 hours",
    pause_tomorrow: "Until tomorrow",
    paused_until: "Notifications paused until",
    update_status: "Update your status",
    set_away: "Set yourself as away",
    set_active: "Set yourself as active",
    resume_notifications: "Resume notifications",
    away: "Away",
    busy: "Busy",
    dnd: "Do not disturb",
    offline_status: "Away",
    status_placeholder: "What's your status?",
    clear_after: "Clear after",
    clear_never: "Don't clear",
    clear_30m: "30 minutes",
    clear_1h: "1 hour",
    clear_4h: "4 hours",
    clear_today: "Today",
    clear_status: "Clear status",
    err_status: "Couldn't change the status",
    for_your_org: "For your organization",
    status_presets: [
        ("🗓️", "In a meeting"),
        ("🚌", "Commuting"),
        ("🤒", "Out sick"),
        ("🌴", "Vacationing"),
        ("🏡", "Working remotely"),
    ],
    p_sound: "Notification sound",
    attach: "Attach a file",
    only_in_app_files: "Sending files works only in the desktop app",
    remove_file: "Remove file",
    uploading: "Uploading…",
    new_message: "New message",
    new_message_to: "To:",
    write_to_colleague: "Message a colleague",
    edit_profile: "Edit",
    local_time: "local time",
    add_channels: "Add channels",
    create_channel: "Create a channel",
    channel_name: "Name",
    channel_name_ph: "e.g. plan-budget",
    channel_desc: "Description (optional)",
    channel_desc_ph: "What is this channel about?",
    next: "Next",
    create: "Create",
    add_people: "Add people",
    add_people_to: "Add people to",
    find_members: "Find members",
    add_btn: "Add",
    search_people_ph: "Enter a name or email",
    skip_for_now: "Skip for now",
    about_channel: "About",
    members_tab: "Members",
    channel_description: "Description",
    no_description: "No description",
    save: "Save",
    save_changes: "Save changes",
    leave_channel: "Leave channel",
    leave_confirm: "You'll stop getting messages from this channel. You can only come back if someone invites you again.",
    admin: "Admin",
    remove_from_channel: "Remove from channel",
    err_create_channel: "Couldn't create the channel",
    err_members: "Couldn't change members",
    err_update_chat: "Couldn't save the changes",
    err_leave: "Couldn't leave the channel",
    err_search: "Couldn't search people",
    only_admins_edit: "Only admins can change the name and members.",
    in_app_only_groups: "Channel management works only in the desktop app",
};

thread_local! {
    static CURRENT: std::cell::Cell<Lang> = const { std::cell::Cell::new(Lang::Ru) };
}

/// Makes the language current for [`tr`].
pub fn set_current(lang: Lang) {
    CURRENT.with(|c| c.set(lang));
}

/// Strings in the current language.
pub fn tr() -> &'static Tr {
    for_lang(CURRENT.with(std::cell::Cell::get))
}

/// Strings of the given language.
pub fn for_lang(lang: Lang) -> &'static Tr {
    match lang {
        Lang::Ru => &RU,
        Lang::En => &EN,
    }
}

impl Tr {
    /// Localized title of a sidebar section by its ID.
    pub fn section(&self, id: &str, fallback: &str) -> String {
        match id {
            "starred" => self.sec_starred,
            "channels" => self.sec_channels,
            "dms" => self.sec_dms,
            "meetings" => self.sec_meetings,
            _ => fallback,
        }
        .to_owned()
    }

    /// Name of a notification sound by its id.
    pub fn sound_name(&self, id: &str) -> &'static str {
        let (ru, en) = match id {
            "marimba" => ("Шунт", "Shunt"),
            "chime" => ("Перезвон", "Chime"),
            "drop" => ("Капля", "Drop"),
            "knock" => ("Тук-тук", "Knock"),
            "glass" => ("Стекло", "Glass"),
            "electric_piano" => ("Электропиано", "Electric piano"),
            "retro" => ("Ретро", "Retro"),
            _ => ("Без звука", "None"),
        };
        match self.lang {
            Lang::Ru => ru,
            Lang::En => en,
        }
    }

    pub fn sidebar_filter(&self, filter: SidebarFilter) -> &'static str {
        match filter {
            SidebarFilter::Active => self.f_active,
            SidebarFilter::Unreads => self.f_unreads,
            SidebarFilter::Mentions => self.f_mentions,
            SidebarFilter::All => self.f_all,
        }
    }

    pub fn sidebar_sort(&self, sort: SidebarSort) -> &'static str {
        match sort {
            SidebarSort::Alphabetical => self.s_az,
            SidebarSort::Recency => self.s_recency,
            SidebarSort::Custom => self.s_custom,
        }
    }

    pub fn notify_level(&self, level: NotifyLevel) -> &'static str {
        match level {
            NotifyLevel::All => self.notify_all,
            NotifyLevel::Mentions => self.notify_mentions,
            NotifyLevel::Nothing => self.notify_nothing,
        }
    }

    pub fn notify_level_notice(&self, level: NotifyLevel) -> String {
        match level {
            NotifyLevel::All => self.notice_notify_all,
            NotifyLevel::Mentions => self.notice_notify_mentions,
            NotifyLevel::Nothing => self.notice_notify_nothing,
        }
        .to_owned()
    }

    pub fn new_replies(&self, n: u32) -> String {
        match self.lang {
            Lang::Ru => format!("{n} {}", ru_plural(n, "новый", "новых", "новых")),
            Lang::En => format!("{n} new"),
        }
    }

    pub fn replies(&self, n: u32) -> String {
        match self.lang {
            Lang::Ru => format!("{n} {}", ru_plural(n, "ответ", "ответа", "ответов")),
            Lang::En => format!("{n} {}", if n == 1 { "reply" } else { "replies" }),
        }
    }

    pub fn leave_title(&self, channel: &str) -> String {
        match self.lang {
            Lang::Ru => format!("Покинуть канал #{channel}?"),
            Lang::En => format!("Leave #{channel}?"),
        }
    }

    /// A moment: `day` ("today", a date) at `time`.
    pub fn day_at(&self, day: &str, time: &str) -> String {
        match self.lang {
            Lang::Ru => format!("{day} в {time}"),
            Lang::En => format!("{day} at {time}"),
        }
    }

    pub fn last_reply(&self, when: &str) -> String {
        match self.lang {
            Lang::Ru => format!("Последний ответ {when}"),
            Lang::En => format!("Last reply {when}"),
        }
    }

    pub fn typing(&self, names: &[String]) -> Option<String> {
        match (self.lang, names.len()) {
            (_, 0) => None,
            (Lang::Ru, 1) => Some(format!("{} печатает…", names[0])),
            (Lang::Ru, _) => Some(format!("{} печатают…", names.join(", "))),
            (Lang::En, 1) => Some(format!("{} is typing…", names[0])),
            (Lang::En, _) => Some(format!("{} are typing…", names.join(", "))),
        }
    }

    pub fn act_mention_in(&self, chat: &str) -> String {
        match self.lang {
            Lang::Ru => format!("Упоминание в {chat}"),
            Lang::En => format!("Mention in {chat}"),
        }
    }

    pub fn act_reply_in(&self, chat: &str) -> String {
        match self.lang {
            Lang::Ru => format!("Ответ в треде · {chat}"),
            Lang::En => format!("Thread reply in {chat}"),
        }
    }

    pub fn act_reacted_in(&self, chat: &str) -> String {
        match self.lang {
            Lang::Ru => format!("на ваше сообщение в {chat}"),
            Lang::En => format!("on your message in {chat}"),
        }
    }

    /// How many people are in a call now.
    pub fn in_call(&self, n: u32) -> String {
        match self.lang {
            Lang::Ru => format!("{n} {} в звонке", ru_plural(n, "человек", "человека", "человек")),
            Lang::En => format!("{n} in the call"),
        }
    }

    pub fn members_count(&self, n: u32) -> String {
        match self.lang {
            Lang::Ru => format!("{n} {}", ru_plural(n, "участник", "участника", "участников")),
            Lang::En => format!("{n} member{}", if n == 1 { "" } else { "s" }),
        }
    }

    /// The reply bar above the message field.
    pub fn replying_to(&self, name: &str) -> String {
        match self.lang {
            Lang::Ru => format!("Ответ: {name}"),
            Lang::En => format!("Replying to {name}"),
        }
    }

    pub fn quoting(&self, name: &str) -> String {
        match self.lang {
            Lang::Ru => format!("Цитата: {name}"),
            Lang::En => format!("Quoting {name}"),
        }
    }

    pub fn posted_in(&self, chat: &str) -> String {
        match self.lang {
            Lang::Ru => format!("Опубликовано в {chat}"),
            Lang::En => format!("Posted in {chat}"),
        }
    }

    pub fn search_in(&self, org: &str) -> String {
        match self.lang {
            Lang::Ru => format!("Поиск: {org}"),
            Lang::En => format!("Search {org}"),
        }
    }

    pub fn message_to_channel(&self, name: &str) -> String {
        match self.lang {
            Lang::Ru => format!("Сообщение в #{name}"),
            Lang::En => format!("Message #{name}"),
        }
    }

    pub fn message_to_person(&self, name: &str) -> String {
        match self.lang {
            Lang::Ru => format!("Сообщение: {name}"),
            Lang::En => format!("Message {name}"),
        }
    }

    /// Who reacted, for the reaction tooltip, as in Slack: "Anna, Boris and you reacted".
    /// `more` is how many others Telemost did not name.
    pub fn reacted(&self, names: &[String], more: u32, mine: bool) -> String {
        let mut who: Vec<String> = names.to_vec();
        if more > 0 {
            who.push(match self.lang {
                Lang::Ru => format!("ещё {more}"),
                Lang::En => format!("{more} more"),
            });
        }
        if mine {
            who.push(match self.lang {
                Lang::Ru => "вы".into(),
                Lang::En => "you".into(),
            });
        }
        let list = self.list(&who);
        match self.lang {
            Lang::Ru => format!("Отреагировали: {list}"),
            Lang::En => format!("{list} reacted"),
        }
    }

    /// The reads of an own message in a group: "Read by 5", or "Read by everyone".
    pub fn read_by(&self, n: u32, everyone: bool) -> String {
        match self.lang {
            _ if everyone => self.read_by_all.to_owned(),
            Lang::Ru => format!("{} {n}", ru_plural(n, "Прочитал(а)", "Прочитали", "Прочитали")),
            Lang::En => format!("Read by {n}"),
        }
    }

    /// Who read a message: the latest readers by name and how many more, "Anna, Boris and 2 more".
    pub fn readers(&self, names: &[String], more: u32) -> String {
        let mut who = names.to_vec();
        if more > 0 && !names.is_empty() {
            who.push(match self.lang {
                Lang::Ru => format!("ещё {more}"),
                Lang::En => format!("{more} more"),
            });
        }
        self.list(&who)
    }

    /// "Anna, Boris and Vera".
    pub fn list(&self, names: &[String]) -> String {
        let and = match self.lang {
            Lang::Ru => " и ",
            Lang::En => " and ",
        };
        match names.split_last() {
            Some((last, rest)) if !rest.is_empty() => format!("{}{and}{last}", rest.join(", ")),
            Some((last, _)) => last.clone(),
            None => String::new(),
        }
    }

    /// What a note about the chat says after the name of its author, as in Slack's "joined
    /// #channel"; `name` names the people it adds or removes. Russian names are not declined,
    /// so they follow a colon.
    pub fn chat_note(&self, note: &SystemNote, author: &UserId, name: impl Fn(&UserId) -> String) -> String {
        let list = |ids: &[UserId]| self.list(&ids.iter().map(&name).collect::<Vec<_>>());
        let only_author = |ids: &[UserId]| ids == std::slice::from_ref(author);
        let ru = self.lang == Lang::Ru;
        match note {
            SystemNote::Created { name } => match (ru, mostik_core::split_chat_icon(name).1) {
                (true, "") => "создал(а) канал".to_owned(),
                (true, name) => format!("создал(а) канал #{name}"),
                (false, "") => "created the channel".to_owned(),
                (false, name) => format!("created #{name}"),
            },
            SystemNote::Joined => self.chat_joined(),
            SystemNote::Left => self.chat_left(),
            SystemNote::Members { added, removed } if removed.is_empty() && only_author(added) => self.chat_joined(),
            SystemNote::Members { added, removed } if added.is_empty() && only_author(removed) => self.chat_left(),
            SystemNote::Members { added, removed } => match (ru, added.is_empty(), removed.is_empty()) {
                (true, false, true) => format!("добавил(а) в канал: {}", list(added)),
                (true, true, _) => format!("удалил(а) из канала: {}", list(removed)),
                (true, false, false) => format!("добавил(а) в канал: {}; удалил(а): {}", list(added), list(removed)),
                (false, false, true) => format!("added {}", list(added)),
                (false, true, _) => format!("removed {}", list(removed)),
                (false, false, false) => format!("added {} and removed {}", list(added), list(removed)),
            },
        }
    }

    fn chat_joined(&self) -> String {
        match self.lang {
            Lang::Ru => "присоединился(-ась) к каналу",
            Lang::En => "joined the channel",
        }
        .to_owned()
    }

    fn chat_left(&self) -> String {
        match self.lang {
            Lang::Ru => "покинул(а) канал",
            Lang::En => "left the channel",
        }
        .to_owned()
    }

    /// How long a call lasted, from whole minutes.
    pub fn call_lasted(&self, minutes: u64) -> String {
        let (h, m) = (minutes / 60, minutes % 60);
        match (self.lang, h, m) {
            (Lang::Ru, 0, 0) => "меньше минуты".to_owned(),
            (Lang::En, 0, 0) => "under a minute".to_owned(),
            (Lang::Ru, 0, _) => format!("длился {m} мин"),
            (Lang::Ru, _, 0) => format!("длился {h} ч"),
            (Lang::Ru, _, _) => format!("длился {h} ч {m} мин"),
            (Lang::En, 0, _) => format!("lasted {m} min"),
            (Lang::En, _, 0) => format!("lasted {h} h"),
            (Lang::En, _, _) => format!("lasted {h} h {m} min"),
        }
    }

    pub fn me_suffix(&self, name: &str) -> String {
        match self.lang {
            Lang::Ru => format!("{name} (вы)"),
            Lang::En => format!("{name} (you)"),
        }
    }

    pub fn intro_dm(&self, name: &str) -> String {
        match self.lang {
            Lang::Ru => format!("Это начало личной переписки с собеседником «{name}»."),
            Lang::En => format!("This conversation is just between you and {name}."),
        }
    }

    pub fn intro_channel_with(&self, description: &str) -> String {
        format!("{} {description}", self.intro_channel)
    }

    pub fn window_title(&self, unread: u32) -> String {
        let base = match self.lang {
            Lang::Ru => "Мостик | неофициальный клиент Телемост",
            Lang::En => "Mostik | unofficial Telemost client",
        };
        if unread > 0 {
            format!("({unread}) {base}")
        } else {
            base.to_owned()
        }
    }

    pub fn sign_out_of(&self, org: &str) -> String {
        match self.lang {
            Lang::Ru => format!("Выйти из «{org}»"),
            Lang::En => format!("Sign out of {org}"),
        }
    }

    /// The "Update" button tooltip and the line above it in Preferences.
    /// "Delete :name:?" in the emoji picker.
    pub fn emoji_delete_ask(&self, name: &str) -> String {
        match self.lang {
            Lang::Ru => format!("Удалить :{name}:?"),
            Lang::En => format!("Delete :{name}:?"),
        }
    }

    pub fn update_available(&self, v: &str) -> String {
        match self.lang {
            Lang::Ru => format!("Доступна новая версия {v}. Мостик установит её и перезапустится."),
            Lang::En => format!("Version {v} is available. Mostik will install it and restart."),
        }
    }

    /// The "Update" button while the new version downloads and installs.
    pub fn updating(&self, percent: Option<u8>) -> String {
        let word = match self.lang {
            Lang::Ru => "Обновление",
            Lang::En => "Updating",
        };
        match percent {
            Some(p) => format!("{word} {p}%"),
            None => format!("{word}…"),
        }
    }

    pub fn version(&self, v: &str) -> String {
        match self.lang {
            Lang::Ru => format!("Версия {v}"),
            Lang::En => format!("Version {v}"),
        }
    }

    pub fn size(&self, bytes: u64) -> String {
        let (b, kb, mb) = match self.lang {
            Lang::Ru => ("Б", "КБ", "МБ"),
            Lang::En => ("B", "KB", "MB"),
        };
        match bytes {
            0 => String::new(),
            x if x < 1024 => format!("{x} {b}"),
            x if x < 1024 * 1024 => format!("{:.0} {kb}", x as f64 / 1024.0),
            x => format!("{:.1} {mb}", x as f64 / 1024.0 / 1024.0),
        }
    }
}

/// The Russian word form for `n`: `one` for 1, 21, 101…, `few` for 2–4, 22–24…, `many` for the
/// rest, 11–14 included.
fn ru_plural(n: u32, one: &'static str, few: &'static str, many: &'static str) -> &'static str {
    let (m10, m100) = (n % 10, n % 100);
    if m10 == 1 && m100 != 11 {
        one
    } else if (2..=4).contains(&m10) && !(12..=14).contains(&m100) {
        few
    } else {
        many
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plurals() {
        assert_eq!(RU.replies(1), "1 ответ");
        assert_eq!(RU.replies(3), "3 ответа");
        assert_eq!(RU.replies(11), "11 ответов");
        assert_eq!(RU.replies(21), "21 ответ");
        assert_eq!(EN.replies(1), "1 reply");
        assert_eq!(EN.replies(2), "2 replies");
        assert_eq!(RU.in_call(1), "1 человек в звонке");
        assert_eq!(RU.in_call(3), "3 человека в звонке");
        assert_eq!(RU.in_call(5), "5 человек в звонке");
        assert_eq!(EN.in_call(2), "2 in the call");
        let members = [1, 2, 5, 11, 12, 21, 22, 112].map(|n| RU.members_count(n));
        assert_eq!(
            members,
            [
                "1 участник",
                "2 участника",
                "5 участников",
                "11 участников",
                "12 участников",
                "21 участник",
                "22 участника",
                "112 участников"
            ]
        );
        assert_eq!(EN.members_count(1), "1 member");
        assert_eq!(EN.members_count(21), "21 members");
    }

    #[test]
    fn read_receipts() {
        assert_eq!(RU.read_by(1, false), "Прочитал(а) 1");
        assert_eq!(RU.read_by(5, false), "Прочитали 5");
        assert_eq!(EN.read_by(3, false), "Read by 3");
        assert_eq!(RU.read_by(9, true), "Прочитали все");
        assert_eq!(EN.read_by(9, true), "Read by everyone");
        let names = ["Anna".to_owned(), "Boris".to_owned(), "Vera".to_owned()];
        assert_eq!(RU.readers(&names, 2), "Anna, Boris, Vera и ещё 2");
        assert_eq!(EN.readers(&names[..1], 0), "Anna");
        assert_eq!(EN.readers(&[], 4), "");
    }

    #[test]
    fn chat_notes() {
        let id = |s: &str| UserId(s.into());
        let name = |u: &UserId| format!("<{}>", u.0);
        let members = |added: &[&str], removed: &[&str]| SystemNote::Members {
            added: added.iter().map(|a| id(a)).collect(),
            removed: removed.iter().map(|r| id(r)).collect(),
        };
        let me = id("a");
        assert_eq!(
            EN.chat_note(&members(&["b", "c", "d"], &[]), &me, name),
            "added <b>, <c> and <d>"
        );
        assert_eq!(
            RU.chat_note(&members(&["b"], &["c"]), &me, name),
            "добавил(а) в канал: <b>; удалил(а): <c>"
        );
        // Adding or removing oneself is joining or leaving.
        assert_eq!(EN.chat_note(&members(&["a"], &[]), &me, name), "joined the channel");
        assert_eq!(RU.chat_note(&members(&[], &["a"]), &me, name), "покинул(а) канал");
        let created = SystemNote::Created {
            name: "⚠️|Test".into()
        };
        assert_eq!(EN.chat_note(&created, &me, name), "created #Test");
    }

    #[test]
    fn moments() {
        assert_eq!(RU.day_at(RU.today_in_text, "10:41"), "сегодня в 10:41");
        assert_eq!(EN.day_at(EN.yesterday_in_text, "9:05 AM"), "yesterday at 9:05 AM");
    }

    #[test]
    fn call_durations() {
        assert_eq!(RU.call_lasted(0), "меньше минуты");
        assert_eq!(EN.call_lasted(0), "under a minute");
        assert_eq!(RU.call_lasted(42), "длился 42 мин");
        assert_eq!(RU.call_lasted(60), "длился 1 ч");
        assert_eq!(EN.call_lasted(120), "lasted 2 h");
        assert_eq!(RU.call_lasted(61), "длился 1 ч 1 мин");
        assert_eq!(EN.call_lasted(95), "lasted 1 h 35 min");
    }

    #[test]
    fn sizes_and_titles() {
        assert_eq!(EN.size(2048), "2 KB");
        assert_eq!(RU.size(5 * 1024 * 1024), "5.0 МБ");
        assert_eq!(EN.window_title(2), "(2) Mostik | unofficial Telemost client");
        assert_eq!(RU.window_title(0), "Мостик | неофициальный клиент Телемост");
        assert_eq!(RU.section("dms", "x"), "Личные сообщения");
    }
}
