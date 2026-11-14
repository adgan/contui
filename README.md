# Dockyard

A native macOS Docker client, written in Rust on [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui) — the UI framework behind Zed. It exists because the Rancher Desktop dashboard is slow and ugly: this one renders on the GPU, has no webview, and updates from the Docker daemon's event stream instead of polling.

First version scope: **containers, images and volumes**, read-only.

## What it does

- Lists every container (running and stopped), image, and volume for the selected Docker host.
- Navigates like Docker Desktop: an icon rail, a sortable table per resource, search, and a detail page per row with **Inspect** and **Logs** tabs.
- Stays live: subscribes to `GET /events` and refreshes within ~250ms of a container starting, stopping or dying. No polling timer.
- Finds any local Docker host: `DOCKER_HOST`, `DOCKER_CONTEXT`, every context in `~/.docker/contexts` (Rancher Desktop, Docker Desktop, Colima, OrbStack all register one), and the common socket paths. A host switcher in the header probes each one on open.
- Tells you the truth when the engine is down: the header chip turns red, a banner names the problem and offers Retry, and the last known rows stay on screen instead of vanishing.

## Not in this version

- **Read-only.** No start / stop / restart / remove / pull / run. Those change the architecture (confirmations, optimistic updates) and get designed on their own.
- No live CPU/memory columns (that needs one `/stats` stream per running container).
- No log following — the Logs tab is a 500-line tail with a refresh button.
- `ssh://` hosts are listed but not connected; no passphrase prompt yet.
- No Compose grouping, no exec shell, no builds, no Kubernetes view.

## Build and run

Requirements: Rust stable (1.99+), Xcode Command Line Tools, and a running Docker engine (tested against Rancher Desktop).

```sh
cargo run
```

For a Finder-launchable app:

```sh
./bundle.sh        # or: just bundle, if you have just
open target/Dockyard.app
```

`bundle.sh` builds release, assembles `Dockyard.app` with `assets/Info.plist`, ad-hoc signs it, and drops it in `target/`. It is not notarized: Gatekeeper will only accept it locally.

## Keyboard

| Keys | Action |
| --- | --- |
| ⌘1 / ⌘2 / ⌘3 | Containers / Images / Volumes |
| ⌘F | Focus search |
| ⌘R | Refresh |
| ↑ / ↓ | Move selection |
| Enter | Open the selected row |
| Esc | Back to the list, or clear the search |

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
