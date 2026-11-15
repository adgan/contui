# contUI

A native macOS Docker client, written in Rust on [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui) — the UI framework behind Zed. It exists because the Rancher Desktop dashboard is slow and ugly: this one renders on the GPU, has no webview, and updates from the Docker daemon's event stream instead of polling.

First version scope: **containers, images and volumes**, read-only.

## What it does

- Lists every container (running and stopped), image, and volume for the selected Docker host.
- Navigates like Docker Desktop: an icon rail, a sortable table per resource, search, and a detail page per row with **Inspect** and **Logs** tabs.
- Stays live: subscribes to `GET /events` and refreshes within ~250ms of a container starting, stopping or dying. No polling timer.
- Shows live CPU and memory for running containers: one `/stats` stream per on-screen container, refreshed every ~2s, with a bar per reading. Stopped containers show `—`; leaving the containers list stops the streams.
- Finds any local Docker host: `DOCKER_HOST`, `DOCKER_CONTEXT`, every context in `~/.docker/contexts` (Rancher Desktop, Docker Desktop, Colima, OrbStack all register one), and the common socket paths. A host switcher in the header probes each one on open.
- Tells you the truth when the engine is down: the header chip turns red, a banner or the empty area names the problem (an unreachable socket and a permission error read differently), Retry and ⌘R both re-ask, and the last known rows stay on screen instead of vanishing. When the engine comes back the list refetches itself — events that happened while it was down are never replayed.

## Doing things

Containers, images and volumes are no longer read-only:

- **Containers** — start, stop, restart, remove. Hover a row for the icons, right-click for the same menu, or use the detail page. Remove always asks first; **Force** is an explicit choice when the container is running.
- **Images** — remove (Force when a container uses it), Run (opens the form pre-filled), and Pull from the header. Pull shows layer progress and leaves the tables live.
- **Volumes** — remove. The daemon's "in use" error stays in the confirm dialog.
- **Run** builds a container from image, name, command, ports, environment and volumes, starts it, and opens its detail page. If the image is not local, the dialog offers **Pull it** and retries the submitted form.
- Nothing is optimistic: a pending row reads `Starting…` / `Removing…`, the event stream confirms, and the list refetches. A failure keeps the last rows on screen and names the daemon's error.

## Not in this version

- **No prune, bulk actions, exec, builds or Compose.** No registry login: private images fail with the daemon's auth error. A pull cannot be cancelled. Stop uses Docker's 10-second grace; there is no separate kill.
- No log following — the Logs tab is a 500-line tail with timestamps and a Reload button.
- `ssh://` hosts are listed but not connected; no passphrase prompt yet.
- No Compose grouping, no exec shell, no builds, no Kubernetes view.

## Install with Homebrew

A public tap carries the app: `brew install --HEAD adgan/tap/contui`. The
[tap README](https://github.com/adgan/homebrew-tap) covers trust, launch, upgrade and
uninstall. It is a source build until the first tagged release, so it needs Rust and the
Xcode Command Line Tools.

## Build and run

Requirements: Rust stable (1.99+), Xcode Command Line Tools, and a running Docker engine (tested against Rancher Desktop).

```sh
cargo run
```

For a Finder-launchable app:

```sh
./bundle.sh        # or: just bundle, if you have just
open target/contUI.app
```

`bundle.sh` builds release, assembles `contUI.app` with `assets/Info.plist`, ad-hoc signs it, and drops it in `target/`. It is not notarized: Gatekeeper will only accept it locally.

## Keyboard

| Keys | Action |
| --- | --- |
| ⌘1 / ⌘2 / ⌘3 | Containers / Images / Volumes |
| ⌘F | Focus search |
| ⌘N | Run a container |
| ⌘⇧P | Pull an image |
| ⌘R | Refresh |
| ⌘Q | Quit |
| ↑ / ↓ | Move selection |
| Enter | Open the selected row |
| Esc | Clear the search, else back to the list |

## Tests

```sh
cargo test --lib                                  # units: discovery, formatting, table, inspect, states
cargo test --test live_docker -- --ignored        # against the running daemon
```

## Layout

```
src/
  docker/    endpoint discovery, bollard service actor, event stream, model mapping
  ui/        rail, header, generic table, detail page, empty states
  theme.rs   every colour in one place
  assets.rs  embedded fonts (Inter, JetBrains Mono — SIL OFL, see assets/fonts/LICENSE-*)
```

The UI thread never touches the network: one tokio runtime owns the Docker client, and responses arrive through a channel that updates the view entity.
