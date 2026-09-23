#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! CSRP Moderation desktop shell.
//!
//! This is a window around the existing staff panel, not a second client. The
//! panel is loaded from the web, so every feature shipped there appears here
//! with no work, and nothing is duplicated in Rust.
//!
//! What the shell adds is the part a browser tab cannot do: staying in the tray
//! so alerts still arrive once the window is closed, being installed and updated
//! as an ordinary application, reopening on the server you were last in, and
//! sending you to your real browser to sign in so Discord is not a fresh login
//! every time.

use std::path::PathBuf;
use std::sync::Mutex;

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, State, Url, WebviewUrl, WebviewWindowBuilder,
};
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_plugin_opener::OpenerExt;

/// The panel this wraps. Override at build time with CSRP_PANEL_URL.
const PANEL_URL: &str = match option_env!("CSRP_PANEL_URL") {
    Some(url) => url,
    None => "https://staff-moderation.officialcaliforniastateroleplay.com/guilds",
};

/// The scheme the site uses to hand a finished sign-in back to this app.
const SCHEME: &str = "csrp-moderation";

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

/// What the shell remembers between launches, and where it is in signing in.
#[derive(Default)]
struct Shell {
    /// True while the browser has the sign-in and the window is on the splash.
    waiting: Mutex<bool>,
    /// The splash page's own address, captured once the window exists, because
    /// it differs per platform and cannot be spelled out ahead of time.
    splash: Mutex<Option<Url>>,
}

#[derive(serde::Serialize)]
struct Start {
    waiting: bool,
    url: String,
}

fn panel() -> Url {
    PANEL_URL.parse().expect("CSRP_PANEL_URL must be a valid URL")
}

fn panel_path(path: &str) -> Url {
    panel().join(path).expect("panel path must be valid")
}

fn is_panel(url: &Url) -> bool {
    url.host_str().is_some() && url.host_str() == panel().host_str()
}

fn is_discord(url: &Url) -> bool {
    match url.host_str() {
        Some(host) => {
            host == "discord.com"
                || host.ends_with(".discord.com")
                || host == "discordapp.com"
                || host.ends_with(".discordapp.com")
        }
        None => false,
    }
}

/// True for the panel's own sign-in page, which belongs in the real browser.
fn is_sign_in(url: &Url) -> bool {
    is_panel(url) && (url.path() == "/login" || url.path().starts_with("/login/"))
}

/// The guild a panel URL is scoped to, if it is scoped to one.
///
/// Every per-server page is `/{guildId}/...`, so the id is the first segment
/// when it looks like a snowflake.
fn guild_of(url: &Url) -> Option<String> {
    if !is_panel(url) {
        return None;
    }

    let first = url.path_segments()?.next()?;
    let digits = (15..=20).contains(&first.len()) && first.chars().all(|c| c.is_ascii_digit());

    digits.then(|| first.to_string())
}

fn state_file(app: &AppHandle) -> Option<PathBuf> {
    let dir = app.path().app_config_dir().ok()?;
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("last-guild"))
}

fn last_guild(app: &AppHandle) -> Option<String> {
    let raw = std::fs::read_to_string(state_file(app)?).ok()?;
    let id = raw.trim().to_string();
    let digits = (15..=20).contains(&id.len()) && id.chars().all(|c| c.is_ascii_digit());

    digits.then_some(id)
}

fn remember_guild(app: &AppHandle, id: &str) {
    if last_guild(app).as_deref() == Some(id) {
        return;
    }
    if let Some(path) = state_file(app) {
        let _ = std::fs::write(path, id);
    }
}

/// Where the window should go when it opens: the server you were last in.
fn start_url(app: &AppHandle) -> Url {
    match last_guild(app) {
        Some(id) => panel_path(&format!("/{id}/dashboard")),
        None => panel_path("/guilds"),
    }
}

fn show_main<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Send the sign-in to the real browser, and park the window on the splash.
///
/// Discord in an embedded webview is a fresh login every time, because the
/// webview has none of the browser's Discord session. Handing it to the default
/// browser makes it one click for anybody already signed in there.
fn open_sign_in(app: &AppHandle) {
    *app.state::<Shell>().waiting.lock().unwrap() = true;

    let _ = app
        .opener()
        .open_url(panel_path("/login?desktop=1").to_string(), None::<&str>);

    let splash = app.state::<Shell>().splash.lock().unwrap().clone();
    if let (Some(url), Some(window)) = (splash, app.get_webview_window("main")) {
        let _ = window.navigate(url);
    }
}

/// Take the token the browser handed back and let the panel set its cookie.
///
/// The session cookie is httpOnly, so the page cannot be given it directly.
/// Navigating to the panel's own callback inside this webview is what puts it
/// in this webview's cookie jar, where it then lasts the full thirty days.
fn finish_sign_in(app: &AppHandle, token: &str) {
    *app.state::<Shell>().waiting.lock().unwrap() = false;

    let mut url = panel_path("/auth");
    url.query_pairs_mut().append_pair("token", token);

    if let Some(window) = app.get_webview_window("main") {
        let _ = window.navigate(url);
    }

    show_main(app);
}

fn handle_deep_link(app: &AppHandle, url: &Url) {
    if url.scheme() != SCHEME {
        return;
    }

    let token = url
        .query_pairs()
        .find(|(key, _)| key == "token")
        .map(|(_, value)| value.into_owned())
        .unwrap_or_default();

    if !token.is_empty() {
        finish_sign_in(app, &token);
    }
}

#[tauri::command]
fn start(app: AppHandle, state: State<Shell>) -> Start {
    let waiting = *state.waiting.lock().unwrap();

    Start {
        waiting,
        url: start_url(&app).to_string(),
    }
}

#[tauri::command]
fn sign_in(app: AppHandle) {
    let _ = app.opener().open_url(
        panel_path("/login?desktop=1").to_string(),
        None::<&str>,
    );
}

fn main() {
    tauri::Builder::default()
        .manage(Shell::default())
        // A second launch focuses the running window rather than opening
        // another. On Windows and Linux a deep link arrives as an argument to
        // that second launch, so it is picked up here too.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            for argument in argv.iter().skip(1) {
                if let Ok(url) = argument.parse::<Url>() {
                    handle_deep_link(app, &url);
                }
            }
            show_main(app);
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .invoke_handler(tauri::generate_handler![start, sign_in])
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

            // Installers register the scheme; a development build has no
            // installer, so it registers itself on the platforms that allow it.
            #[cfg(any(windows, target_os = "linux"))]
            let _ = app.deep_link().register_all();

            let opened = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                for url in event.urls() {
                    handle_deep_link(&opened, &url);
                }
            });

            // The window opens on the local splash, which asks the shell where
            // to go. Going straight to the panel would mean nowhere to return
            // to when the sign-in is handed to the browser.
            let navigating = app.handle().clone();
            let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .title("CSRP Moderation")
                .inner_size(1280.0, 800.0)
                .min_inner_size(960.0, 600.0)
                .initialization_script(NOTIFICATION_SHIM)
                .on_navigation(move |url| {
                    if let Some(id) = guild_of(url) {
                        remember_guild(&navigating, &id);
                    }

                    if !is_sign_in(url) && !is_discord(url) {
                        return true;
                    }

                    // Cancelling leaves the webview where it is, so the splash
                    // is put back in front of it from outside this callback.
                    let handle = navigating.clone();
                    let _ = navigating.run_on_main_thread(move || open_sign_in(&handle));

                    false
                })
                .build()?;

            *app.state::<Shell>().splash.lock().unwrap() = Some(window.url()?);

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
