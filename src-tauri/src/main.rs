#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! CSRP Moderation desktop shell.
//!
//! This is a window around the existing staff panel, not a second client. The
//! panel is loaded from the web, so every feature shipped there appears here
//! with no work, and nothing is duplicated in Rust.
//!
//! Two things the browser cannot do are the reason this exists: staying in the
//! tray so alerts still arrive once the window is closed, and being installed
//! and updated as an ordinary application.

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, WebviewUrl, WebviewWindowBuilder,
};

/// The panel this wraps. Override at build time with CSRP_PANEL_URL.
const PANEL_URL: &str = match option_env!("CSRP_PANEL_URL") {
    Some(url) => url,
    None => "https://staff-moderation.officialcaliforniastateroleplay.com/guilds",
};

/// Teaches the page's own `Notification` calls to raise a native notification.
///
/// The panel already asks for permission and posts notifications through the
/// web API. Webviews support that unevenly, and on macOS not at all, so the
/// constructor is replaced with one that forwards to the Tauri plugin. The
/// panel needs no change, and behaves the same in a browser.
const NOTIFICATION_SHIM: &str = r#"
(function () {
  if (!window.__TAURI__ || !window.__TAURI__.notification) return;

  const plugin = window.__TAURI__.notification;

  function show(title, options) {
    const body = (options && options.body) || '';
    plugin
      .isPermissionGranted()
      .then((granted) => (granted ? true : plugin.requestPermission().then((p) => p === 'granted')))
      .then((allowed) => {
        if (allowed) plugin.sendNotification({ title: String(title), body: String(body) });
      })
      .catch(() => {});
  }

  class DesktopNotification {
    constructor(title, options) {
      this.title = title;
      this.body = (options && options.body) || '';
      // The panel assigns onclick; focusing is handled by the tray shell, so
      // the handler is kept callable rather than wired to the native toast.
      this.onclick = null;
      show(title, options);
    }

    close() {}

    static requestPermission() {
      return plugin
        .requestPermission()
        .then((p) => (p === 'granted' ? 'granted' : 'denied'))
        .catch(() => 'denied');
    }
  }

  Object.defineProperty(DesktopNotification, 'permission', {
    get() {
      // The page reads this synchronously, so the last known answer is served
      // and refreshed in the background.
      return window.__csrpNotificationPermission || 'default';
    },
  });

  plugin
    .isPermissionGranted()
    .then((granted) => {
      window.__csrpNotificationPermission = granted ? 'granted' : 'default';
    })
    .catch(() => {});

  window.Notification = DesktopNotification;
})();
"#;

fn main() {
    tauri::Builder::default()
        // A second launch focuses the running window rather than opening another.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            show_main(app);
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(|app| {
            let open = MenuItem::with_id(app, "open", "Open CSRP Moderation", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &quit])?;

            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("CSRP Moderation")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => show_main(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_main(tray.app_handle());
                    }
                })
                .build(app)?;

            let url = PANEL_URL
                .parse()
                .expect("CSRP_PANEL_URL must be a valid URL");

            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
                .title("CSRP Moderation")
                .inner_size(1280.0, 800.0)
                .min_inner_size(960.0, 600.0)
                .initialization_script(NOTIFICATION_SHIM)
                .build()?;

            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing hides the window instead of quitting, which is the whole
            // point: alerts keep arriving while it sits in the tray. Quit is on
            // the tray menu.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("failed to start CSRP Moderation");
}

fn show_main<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
