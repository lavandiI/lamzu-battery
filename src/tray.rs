//! Tray icon and context menu.
//!
//! The whole UI lives on the main thread because `TrayIcon` and the menu items
//! are `!Send`. Worker threads send [`UiCommand`] values and wake the loop with
//! a posted message; menu clicks travel back as [`MenuAction`].

use std::sync::mpsc::{Receiver, Sender};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU32},
};

use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use crate::i18n::{self, Key, Language};
use crate::icons::{IconRenderer, Rgba};
use crate::win::{self, LoopControl};

/// Something the user asked for in the tray menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    /// Poll the battery now.
    Refresh,
    /// Toggle the autostart entry.
    ToggleAutostart,
    /// Open the data directory.
    OpenDataFolder,
    /// Ask Explorer to pin the icon to the taskbar.
    ShowTrayIcon,
    /// Open the release page.
    OpenReleasePage,
    /// Toggle automatic update installation.
    ToggleAutoUpdate,
    /// Check for updates now.
    CheckUpdates,
    /// Install the available update.
    InstallUpdate,
    /// Switch the interface language.
    SetLanguage(Language),
    /// Quit the application.
    Quit,
}

/// Everything the tray icon displays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiSnapshot {
    /// Model-aware name shown as the first, disabled menu item.
    pub display_name: String,
    /// Status line shown as the second, disabled menu item.
    pub status: String,
    /// Tooltip of the tray icon.
    pub tooltip: String,
    /// Text drawn on the icon.
    pub icon_text: String,
    /// Colour of the text.
    pub icon_color: Rgba,
    /// Whether autostart is enabled.
    pub autostart: bool,
    /// Whether updates are installed automatically.
    pub auto_update: bool,
    /// Version that can be installed, if any.
    pub available_version: Option<String>,
    /// Current interface language.
    pub language: Language,
}

/// Message from a worker thread to the UI thread.
#[derive(Debug, Clone)]
pub enum UiCommand {
    /// Re-render the icon, tooltip and menu.
    Snapshot(UiSnapshot),
    /// Show a transient tooltip (download progress).
    Tooltip(String),
    /// Stop the application.
    Quit,
}

fn action_for(identifier: &str) -> Option<MenuAction> {
    match identifier {
        "refresh" => Some(MenuAction::Refresh),
        "autostart" => Some(MenuAction::ToggleAutostart),
        "logs" => Some(MenuAction::OpenDataFolder),
        "show_icon" => Some(MenuAction::ShowTrayIcon),
        "release" => Some(MenuAction::OpenReleasePage),
        "auto_update" => Some(MenuAction::ToggleAutoUpdate),
        "check" => Some(MenuAction::CheckUpdates),
        "install" => Some(MenuAction::InstallUpdate),
        "lang_ru" => Some(MenuAction::SetLanguage(Language::Ru)),
        "lang_en" => Some(MenuAction::SetLanguage(Language::En)),
        "exit" => Some(MenuAction::Quit),
        _ => None,
    }
}

/// Build the context menu for a snapshot.
///
/// Every item is bound to a local so it lives until the menu has copied it; the
/// menu is rebuilt only when its labels or check states change, so an open popup
/// is never destroyed underneath the user.
fn build_menu(snapshot: &UiSnapshot) -> Result<Menu, String> {
    let language = snapshot.language;
    let text = |key: Key| i18n::tr(language, key);

    let name = MenuItem::with_id("name", &snapshot.display_name, false, None);
    let status = MenuItem::with_id("status", &snapshot.status, false, None);
    let first_separator = PredefinedMenuItem::separator();
    let refresh = MenuItem::with_id("refresh", text(Key::MenuRefresh), true, None);
    let second_separator = PredefinedMenuItem::separator();
    let autostart = CheckMenuItem::with_id(
        "autostart",
        text(Key::MenuAutostart),
        true,
        snapshot.autostart,
        None,
    );
    let logs = MenuItem::with_id("logs", text(Key::MenuLogs), true, None);
    let show_icon = MenuItem::with_id("show_icon", text(Key::MenuShowIcon), true, None);
    let release = MenuItem::with_id("release", text(Key::MenuReleasePage), true, None);
    let auto_update = CheckMenuItem::with_id(
        "auto_update",
        text(Key::MenuAutoUpdate),
        true,
        snapshot.auto_update,
        None,
    );
    let check = MenuItem::with_id("check", text(Key::MenuCheckUpdates), true, None);
    let install = snapshot.available_version.as_deref().map(|version| {
        MenuItem::with_id(
            "install",
            i18n::tr_args(language, Key::MenuInstallUpdate, &[("version", version)]),
            true,
            None,
        )
    });
    let language_ru = CheckMenuItem::with_id(
        "lang_ru",
        Language::Ru.name(),
        true,
        language == Language::Ru,
        None,
    );
    let language_en = CheckMenuItem::with_id(
        "lang_en",
        Language::En.name(),
        true,
        language == Language::En,
        None,
    );
    let languages =
        Submenu::with_items(text(Key::MenuLanguage), true, &[&language_ru, &language_en])
            .map_err(|error| error.to_string())?;
    let third_separator = PredefinedMenuItem::separator();
    let exit = MenuItem::with_id("exit", text(Key::MenuExit), true, None);

    let menu = Menu::new();
    let append = |item: &dyn tray_icon::menu::IsMenuItem| {
        menu.append(item).map_err(|error| error.to_string())
    };

    append(&name)?;
    append(&status)?;
    append(&first_separator)?;
    append(&refresh)?;
    append(&second_separator)?;
    append(&autostart)?;
    append(&logs)?;
    append(&show_icon)?;
    append(&release)?;
    append(&auto_update)?;
    append(&check)?;
    if let Some(install) = install.as_ref() {
        append(install)?;
    }
    append(&languages)?;
    append(&third_separator)?;
    append(&exit)?;
    Ok(menu)
}

struct TrayState {
    tray: TrayIcon,
    renderer: IconRenderer,
    snapshot: UiSnapshot,
}

impl TrayState {
    fn new(snapshot: UiSnapshot) -> Result<Self, String> {
        let mut renderer = IconRenderer::new();
        let rgba = renderer.render(&snapshot.icon_text, snapshot.icon_color);
        let icon = Icon::from_rgba(rgba, crate::ICON_SIZE, crate::ICON_SIZE)
            .map_err(|error| error.to_string())?;
        let menu = build_menu(&snapshot)?;
        let tray = TrayIconBuilder::new()
            .with_icon(icon)
            .with_tooltip(&snapshot.tooltip)
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(true)
            .with_menu_on_right_click(true)
            .build()
            .map_err(|error| error.to_string())?;
        Ok(Self {
            tray,
            renderer,
            snapshot,
        })
    }

    fn apply(&mut self, command: UiCommand) -> bool {
        match command {
            UiCommand::Quit => return true,
            UiCommand::Tooltip(text) => {
                if let Err(error) = self.tray.set_tooltip(Some(&text)) {
                    log::debug!("could not update the tooltip: {error}");
                }
            }
            UiCommand::Snapshot(snapshot) => self.apply_snapshot(snapshot),
        }
        false
    }

    fn apply_snapshot(&mut self, snapshot: UiSnapshot) {
        let icon_changed = snapshot.icon_text != self.snapshot.icon_text
            || snapshot.icon_color != self.snapshot.icon_color;
        if icon_changed {
            let rgba = self
                .renderer
                .render(&snapshot.icon_text, snapshot.icon_color);
            match Icon::from_rgba(rgba, crate::ICON_SIZE, crate::ICON_SIZE) {
                Ok(icon) => {
                    if let Err(error) = self.tray.set_icon(Some(icon)) {
                        log::debug!("could not update the tray icon: {error}");
                    }
                }
                Err(error) => log::debug!("could not build the tray icon: {error}"),
            }
        }
        if snapshot.tooltip != self.snapshot.tooltip
            && let Err(error) = self.tray.set_tooltip(Some(&snapshot.tooltip))
        {
            log::debug!("could not update the tooltip: {error}");
        }
        let menu_changed = snapshot.display_name != self.snapshot.display_name
            || snapshot.status != self.snapshot.status
            || snapshot.autostart != self.snapshot.autostart
            || snapshot.auto_update != self.snapshot.auto_update
            || snapshot.available_version != self.snapshot.available_version
            || snapshot.language != self.snapshot.language;
        if menu_changed {
            match build_menu(&snapshot) {
                Ok(menu) => self.tray.set_menu(Some(Box::new(menu))),
                Err(error) => log::debug!("could not rebuild the tray menu: {error}"),
            }
        }
        self.snapshot = snapshot;
    }
}

/// Run the tray UI on the calling thread until a quit command arrives.
///
/// `thread_id` receives the id of this thread so workers can wake the loop,
/// `ready` is set once the icon exists and `actions` receives menu clicks.
pub fn run(
    commands: Receiver<UiCommand>,
    actions: Sender<MenuAction>,
    initial: UiSnapshot,
    thread_id: &Arc<AtomicU32>,
    ready: &Arc<AtomicBool>,
) -> Result<(), String> {
    let mut state = TrayState::new(initial)?;
    thread_id.store(
        win::current_thread_id(),
        std::sync::atomic::Ordering::Relaxed,
    );
    ready.store(true, std::sync::atomic::Ordering::Relaxed);
    win::ensure_message_queue();
    log::info!("tray icon is ready");

    let stopped = win::run_message_loop(
        || {
            while let Ok(command) = commands.try_recv() {
                if state.apply(command) {
                    return LoopControl::Quit;
                }
            }
            LoopControl::Continue
        },
        || {
            while let Ok(event) = MenuEvent::receiver().try_recv() {
                if let Some(action) = action_for(&event.id.0) {
                    log::debug!("menu action: {action:?}");
                    let _ = actions.send(action);
                }
            }
            LoopControl::Continue
        },
    );
    log::info!("tray message loop finished (stopped={stopped})");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(language: Language, available: Option<&str>) -> UiSnapshot {
        UiSnapshot {
            display_name: "LAMZU Thorn".to_owned(),
            status: "42% · discharging".to_owned(),
            tooltip: "LAMZU Thorn: 42% (discharging)".to_owned(),
            icon_text: "42".to_owned(),
            icon_color: Rgba([0, 122, 255, 255]),
            autostart: false,
            auto_update: true,
            available_version: available.map(str::to_owned),
            language,
        }
    }

    #[test]
    fn menu_contains_every_item() {
        let menu = build_menu(&snapshot(Language::En, None)).expect("menu");
        // name, status, separator, refresh, separator, autostart, logs, show icon,
        // release, auto-update, check, language, separator, exit.
        assert_eq!(menu.items().len(), 14);
    }

    #[test]
    fn install_item_appears_only_when_an_update_exists() {
        let menu = build_menu(&snapshot(Language::En, Some("1.7.0"))).expect("menu");
        assert_eq!(menu.items().len(), 15);
        let install = menu
            .items()
            .iter()
            .filter_map(|item| item.as_menuitem())
            .find(|item| item.id().0 == "install")
            .map(MenuItem::text);
        assert_eq!(install.as_deref(), Some("Install 1.7.0"));
    }

    #[test]
    fn language_submenu_uses_the_selected_language() {
        let menu = build_menu(&snapshot(Language::Ru, None)).expect("menu");
        let language = menu
            .items()
            .iter()
            .find_map(|item| item.as_submenu())
            .map(Submenu::text);
        assert_eq!(language.as_deref(), Some("Язык"));

        let menu = build_menu(&snapshot(Language::En, None)).expect("menu");
        let language = menu
            .items()
            .iter()
            .find_map(|item| item.as_submenu())
            .map(Submenu::text);
        assert_eq!(language.as_deref(), Some("Language"));
    }

    #[test]
    fn check_state_follows_the_snapshot() {
        let menu = build_menu(&snapshot(Language::En, None)).expect("menu");
        let checks: Vec<bool> = menu
            .items()
            .iter()
            .filter_map(|item| item.as_check_menuitem())
            .map(CheckMenuItem::is_checked)
            .collect();
        // Autostart is off, automatic updates are on.
        assert_eq!(checks, vec![false, true]);

        let items = menu.items();
        let languages = items
            .iter()
            .find_map(|item| item.as_submenu())
            .expect("language submenu");
        let language_checks: Vec<bool> = languages
            .items()
            .iter()
            .filter_map(|item| item.as_check_menuitem())
            .map(CheckMenuItem::is_checked)
            .collect();
        // English is selected, Russian is not.
        assert_eq!(language_checks, vec![false, true]);
    }

    #[test]
    fn identifiers_map_to_actions() {
        assert_eq!(action_for("refresh"), Some(MenuAction::Refresh));
        assert_eq!(
            action_for("lang_ru"),
            Some(MenuAction::SetLanguage(Language::Ru))
        );
        assert_eq!(
            action_for("lang_en"),
            Some(MenuAction::SetLanguage(Language::En))
        );
        assert_eq!(action_for("exit"), Some(MenuAction::Quit));
        assert_eq!(action_for("unknown"), None);
    }
}
