# contUI Homebrew tap — design

Date: 2026-11-14
Status: approved 2026-11-14
Parent spec: `2026-11-14-docker-gpui-client-design.md`

## Intent

Make contUI installable with `brew install` instead of `git clone && cargo build`.
The parent spec's packaging section stops at `just bundle`, which is fine for the author
and useless for anyone else. This adds the tap the app ships through.

The app does not exist yet (`src/` is empty, no tags, no releases). So this design covers
what can be true today — a tap, a source-build formula, and checks that run without a
binary — and names the two things that must wait for the first build: bottles and a cask.

## Goals

1. `brew install --HEAD adgan/tap/contui` works the moment Task 1 of the parent plan
   lands, with no release and no manual steps.
2. When `v0.1.0` is tagged, the formula gains a stable `url` + sha256 and plain
   `brew install adgan/tap/contui` works, still building from source.
3. The tap is checkable in CI from day one: `brew audit --strict` and `brew style` on
   every push and PR.
4. The tap README is the whole story: install, upgrade, uninstall, and how to release.

## Non-goals (now)

- **No bottles.** A bottle is a prebuilt Cellar tarball; producing one requires a working
  build, and the tap workflow that builds, uploads and merges a bottle block cannot be
  validated until then. It becomes a parent-plan task.
- **No cask.** A cask installs a prebuilt `.app` into `/Applications`; there is no release
  artifact and the app is ad-hoc signed with no notarization. A cask that downloads
  nothing, or that Gatekeeper blocks, is worse than none.
- **No automation in `docker-ui`.** No release workflow, no cross-repo token, no
  `repository_dispatch`. The release checklist is five commands in the tap README.
- No changes to app code, the parent design's scope, or the parent plan's tasks beyond an
  added Task 10 and a README line.

## Anti-goals

- A formula whose `url` 404s, or points at a tarball with no `src/`, so `brew install`
  fails with a Cargo error. That is why the formula is HEAD-only until a tag exists.
- A cask or formula that claims to install a GUI app and leaves the user with a binary
  and no way to launch it.
- CI that runs `brew install` and therefore compiles GPUI on every PR.
- A tap that silently drifts from the app: version, license and bundle steps are stated
  in one place each, and the README says where.

## Constraints

- Tap repo: `github.com/adgan/homebrew-tap`, public. Homebrew requires the `homebrew-`
  prefix for a tap named `adgan/tap`.
- Tap layout: `Formula/contui.rb`, `README.md`, `.github/workflows/audit.yml`.
- Source repo: `github.com/adgan/contui`, branch `main`, binary `contui`, bundle
  `contUI.app`, `CFBundleIdentifier dev.local.contui`.
- License: MIT (the repo currently names none; this design sets it).
- Platform: macOS only. arm64 is the tested target; Intel builds are allowed by the
  formula but untested, matching the parent spec.
- No notarization. Ad-hoc codesign inside the build, as the parent spec's `just bundle`
  already does.
- Build needs Rust plus the Xcode command line tools. The formula declares Rust as a
  build dependency; the CLT requirement goes in the README and caveats.

## Approaches considered

The question was what artifact the tap carries while the app is unbuilt.

**1. Source-build formula, HEAD-only (chosen).** `head` points at `main`. Homebrew
allows a head-only formula, so `brew install --HEAD` works before any tag. Cost: the
default `brew install` path is unavailable until `v0.1.0`, and the formula's `install`
duplicates the bundle steps from `just bundle`. Buys: something real and testable today,
which is exactly the state of the project.

**2. Stable formula pinned to `main`'s tarball.** Gives plain `brew install` today.
Rejected: the tarball has no `src/`, so every install fails in `cargo build`; the version
number would be a lie and the sha would change on every push.

**3. Cask around a hand-built `.app` release.** Right long-term shape for a GUI app.
Rejected for now: needs a release artifact, and with no notarization the installed app
trips Gatekeeper. Deferred, not dropped.

Formula-versus-cask, decided for the record: a source-build formula is the only artifact
that works pre-release; the cask becomes the recommended path once a notarized or
user-approved release exists, and the formula stays as the `--HEAD` developer path.

## Architecture

```
adgan/homebrew-tap
  Formula/contui.rb      HEAD-only source build; stable url added at v0.1.0
  README.md                install / upgrade / uninstall / release checklist
  .github/workflows/audit.yml   brew audit --strict + brew style on push and PR
```

`Formula/contui.rb`:

- `desc`, `homepage` (`adgan/contui`), `license "MIT"`, `version "0.1.0"` (documentation
  only until a stable url exists).
- `head "https://github.com/adgan/contui.git", branch: "main"`.
- `depends_on "rust" => :build`, `depends_on :macos`.
- `install`: `cargo build --release`; assemble `contUI.app/Contents/{MacOS,Resources}`
  with the binary as `Contents/MacOS/contUI`; `prefix.install "contUI.app"`;
  `bin.install_symlink prefix/"contUI.app/Contents/MacOS/contUI" => "contui"`;
  ad-hoc `codesign --force --deep --sign -`.
- `caveats`: how to symlink the bundle into `/Applications`, and that Gatekeeper may
  require a right-click-open because there is no notarization.
- `test`: assert the bundle executable exists and is executable. Never launch the GUI.

CI (`audit.yml`) runs on `macos-latest`: checkout, `brew tap adgan/tap "$GITHUB_WORKSPACE"`,
`brew audit --strict --online adgan/tap/contui`, `brew style`. It does not install.

Parent repo changes: one new plan task (Task 10) covering the tag, stable url, bottle, and
cask follow-ups; one line in Task 9's README section pointing at the tap.

## What's left to the builder

- Exact formula `install` shell lines and whether `cargo` needs an explicit path on the
  runner.
- Whether `brew audit --strict` tolerates a head-only formula; if it does not, the audit
  step narrows to what passes and the deviation is recorded in the tap README.
- Test assertion shape, caveat wording, and the release checklist's exact commands.
- Whether the tap README links the parent repo's design/plan docs.
