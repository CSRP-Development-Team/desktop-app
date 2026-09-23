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
* **Reopens where you were.** The last server you had open is remembered and
  loaded on the next launch, instead of the server picker.
* **Signs in through your real browser**, so Discord is not a fresh login every
  time.

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

## How signing in works

An embedded webview has none of your browser's Discord session, so signing in
inside the window means a full Discord login every time. Instead the window
never shows the sign-in: any navigation to the panel's `/login`, or to Discord
itself, is cancelled and `/login?desktop=1` is opened in your default browser,
where you are already signed in.

The site treats that flag as a desktop sign-in: rather than signing the browser
in, its callback hands the session token back over the `csrp-moderation://`
scheme. The app receives it and loads the panel's own callback inside the
window, which is what puts the session cookie in this webview's jar. It lasts
the usual thirty days, so this happens once rather than on every launch.

This needs the matching site change, which lives in the frontend repository in
`src/routes/login/+page.server.ts` and `src/routes/auth/+server.ts`. Without it
the browser signs itself in and the app stays signed out.

The token travels through a URL handed to the operating system, which is the
same exposure the existing web callback already has, but it does mean a desktop
sign-in should not be done on a shared machine.

## Which server it opens

Every per-server page is `/{guildId}/...`, so the shell watches navigation and
writes the current guild id to `last-guild` in its config directory. The next
launch opens that server's dashboard. Deleting the file, or never having opened
a server, falls back to the server picker.

## How notifications work

Webviews support the web `Notification` API unevenly, and macOS does not support
it at all, so the shell replaces `window.Notification` with a shim that calls
Tauri's notification plugin. The panel is unchanged and still behaves normally
in a browser; only inside this shell are its notifications rerouted.

Alerts arrive whenever the app is running, including from the tray. They do not
arrive when it is fully quit — the panel holds a live connection rather than
using web push, so a closed app receives nothing.
