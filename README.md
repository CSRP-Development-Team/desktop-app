# CSRP Moderation desktop

A desktop window around the hosted staff panel, built with [Tauri](https://tauri.app).

It is deliberately not a second client. The panel is loaded from the web, so
anything shipped there appears here with no work and nothing is reimplemented.
What the wrapper adds is the part a browser cannot do:

* **Lives in the tray.** Closing the window hides it, so alerts keep arriving
  when the window is shut. Quit is on the tray menu.
* **Native notifications.** The panel's existing `Notification` calls are
  forwarded to the OS, attributed to this app rather than to a browser.
* **Installed like an application**, with its own icon, and able to start with
  the machine.
* **One instance.** Launching again focuses the running window.

## Status

Prototype. The project is complete and its configuration is valid, but it has
**not been compiled** — it was written on a machine with no Rust toolchain and
no webview libraries. Expect to fix small things on the first real build.

## Building

Needs [Rust](https://rustup.rs) and Node 22. Each installer must be built on the
operating system it targets, because Tauri compiles against that system's
webview, so use the GitHub Actions workflow for Windows and macOS builds.

```bash
npm install
npm run dev      # run against the live panel
npm run build    # produce an installer for the current platform
```

Push a `v*` tag, or run the workflow by hand, to build all three platforms.

## Pointing it somewhere else

The panel URL is baked in at compile time and defaults to the production panel.
Override it to build against staging or a local server:

```bash
CSRP_PANEL_URL=http://localhost:3000/guilds npm run build
```

The same URL has to be allowed in `src-tauri/capabilities/default.json`, which
is what lets the page talk to the shell at all.

## How notifications work

Webviews support the web `Notification` API unevenly, and macOS does not support
it at all, so the shell replaces `window.Notification` with a shim that calls
Tauri's notification plugin. The panel is unchanged and still behaves normally
in a browser; only inside this shell are its notifications rerouted.

Alerts arrive whenever the app is running, including from the tray. They do not
arrive when it is fully quit — the panel holds a live connection rather than
using web push, so a closed app receives nothing.
