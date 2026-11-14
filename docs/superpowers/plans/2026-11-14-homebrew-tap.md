# contUI Homebrew Tap Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a public Homebrew tap so `brew install --HEAD adgan/tap/contui` works as soon as the app's source exists.

**Architecture:** A second public repo, `adgan/homebrew-tap`, holding one HEAD-only source-build formula, an audit workflow, and a README. The rename of `adgan/docker-ui` to `adgan/contui` happens first so the formula's `head` URL is live.

**Tech Stack:** Homebrew 7 (formula Ruby DSL), GitHub CLI, GitHub Actions on `macos-15`.

**Spec:** `docs/superpowers/specs/2026-11-14-homebrew-tap-design.md`

## Global Constraints

- Tap repo `adgan/homebrew-tap`, public; tap name `adgan/tap`; formula file `Formula/contui.rb`; class `Contui < Formula`.
- Product display name `contUI`; binary `contui`; bundle `contUI.app`; bundle id `dev.local.contui`; source repo `adgan/contui`.
- Formula is HEAD-only until a `v*` tag exists. No stable `url`, no sha256, no bottle block, no cask.
- `depends_on "rust" => :build`, `depends_on :macos`.
- License MIT.
- No `brew install` or `cargo build` in CI — audit and style only.
- Tagging, bottles, cask and version bumps are out of scope; they belong to Task 10, added to the parent plan.
- Commits in `adgan/contui` must disable signing on the command line if 1Password is locked: `git -c commit.gpgsign=false commit …` (the repo sets `commit.gpgsign=true`).

## Review Focus

Inputs and conditions the spec implies but no happy path covers:

1. `brew install` without `--HEAD` must fail with a clear "no stable url" message, not download something broken — the formula has no stable `url` (Task 2).
2. A fresh machine has no trust for a third-party tap; `brew audit` and `brew install` refuse until `brew trust adgan/tap` — README must say so, and CI must trust explicitly (Tasks 2, 3).
3. Pre-source HEAD (`src/` absent) makes `cargo build` fail; the failure must be Cargo's, not the formula's missing file copies — guard the `assets/` copies so the formula only fails in Cargo (Task 2).
4. The app is ad-hoc signed; Gatekeeper blocks the first launch after install — caveats and README must state right-click → Open (Task 2, Task 4).
5. `cargo` is not on `PATH` on the author's machine but is on CI; the formula must call the build without depending on an inherited `PATH` shim (Task 2).

---

### Task 1: Rename the GitHub repository to `contui`

**Files:**
- No file changes; repository metadata and local remote only.

**Interfaces:**
- Consumes: nothing.
- Produces: `github.com/adgan/contui` serving the same `main` and `feat/dockyard` branches; local `origin` pointing at the new name.

- [ ] **Step 1: Confirm nothing references the old repo name in a way a rename breaks**

Run: `gh pr list -R adgan/docker-ui --state all && gh release list -R adgan/docker-ui && git ls-remote --tags origin`
Expected: all three empty. (Verified 2026-11-14: no PRs, no releases, no tags.)

- [ ] **Step 2: Rename**

Run: `gh repo rename contui -R adgan/docker-ui --yes`
Expected: `✓ Renamed repository adgan/contui`.

- [ ] **Step 3: Update the local remote and verify**

```bash
git remote set-url origin git@github.com:adgan/contui.git
git remote -v
git fetch origin
gh repo view adgan/contui --json nameWithOwner,url,visibility
```

Expected: both URLs read `adgan/contui`, `git fetch` succeeds, and the JSON shows `"nameWithOwner":"adgan/contui"` with `"visibility":"PUBLIC"`.

- [ ] **Step 4: Verify the old URL still redirects**

Run: `gh repo view adgan/docker-ui --json nameWithOwner`
Expected: JSON reports `adgan/contui` (GitHub redirects renamed repos).

---

### Task 2: Create the tap repo and the formula

**Files:**
- Create in `adgan/homebrew-tap` (local clone at `../homebrew-tap`): `Formula/contui.rb`, `README.md`

**Interfaces:**
- Consumes: `adgan/contui` from Task 1.
- Produces: tap `adgan/tap`; formula `adgan/tap/contui`; install paths `#{prefix}/contUI.app` and `#{bin}/contui`; caveat text naming `/Applications/contUI.app`.

- [ ] **Step 1: Create the empty public repo**

Run: `gh repo create adgan/homebrew-tap --public --description "Homebrew tap for contUI"`
Expected: `✓ Created repository adgan/homebrew-tap`.

- [ ] **Step 2: Clone it beside the app repo**

```bash
cd .. && git clone git@github.com:adgan/homebrew-tap.git
cd homebrew-tap
```

Expected: empty repo with only `.git`.

- [ ] **Step 3: Write `Formula/contui.rb`**

```ruby
# typed: false
# frozen_string_literal: true

class Contui < Formula
  desc "Fast native Docker client for macOS"
  homepage "https://github.com/adgan/contui"
  version "0.1.0"
  license "MIT"

  head "https://github.com/adgan/contui.git", branch: "main"

  depends_on "rust" => :build
  depends_on :macos

  def install
    system "cargo", "build", "--release", "--locked"

    app = buildpath/"contUI.app"
    (app/"Contents/MacOS").mkpath
    (app/"Contents/Resources").mkpath
    cp "target/release/contui", app/"Contents/MacOS/contUI"
    if (buildpath/"assets/Info.plist").exist?
      cp "assets/Info.plist", app/"Contents/Info.plist"
    end
    %w[fonts icons].each do |dir|
      cp_r "assets/#{dir}", app/"Contents/Resources/" if (buildpath/"assets/#{dir}").exist?
    end

    system "codesign", "--force", "--deep", "--sign", "-", app
    prefix.install app
    bin.install_symlink prefix/"contUI.app/Contents/MacOS/contUI" => "contui"
  end

  def caveats
    <<~EOS
      contUI installs as an app bundle. To launch it from Finder or Spotlight:
        ln -sf #{opt_prefix}/contUI.app /Applications/contUI.app

      The bundle is ad-hoc signed and not notarized, so the first launch may
      need right-click → Open.
    EOS
  end

  test do
    assert_predicate prefix/"contUI.app/Contents/MacOS/contUI", :executable?
  end
end
```

Note: `--locked` requires the app repo to commit `Cargo.lock` (parent plan Task 1 does). Nothing is built in this task.

- [ ] **Step 4: Write a placeholder `README.md`**

```markdown
# Homebrew tap for contUI

Work in progress: the formula is HEAD-only and builds from source.
Installation is documented once the app's first task lands.
```

- [ ] **Step 5: Commit and push**

```bash
git add Formula/contui.rb README.md
git commit -m "feat: HEAD-only contui formula"
git push -u origin HEAD:main
```

Expected: `main` exists on `adgan/homebrew-tap`.

- [ ] **Step 6: Verify the formula through a real tap**

```bash
brew tap adgan/tap
brew trust adgan/tap
brew audit --strict adgan/tap/contui
brew style adgan/tap/contui
```

Expected: `brew audit --strict` prints nothing but the rubocop-md config warning; `brew style` reports `no offenses detected`. (Verified 2026-11-14 against a probe formula: a head-only formula passes `--strict`.)

- [ ] **Step 7: Verify the source actually resolves**

Run: `brew fetch --HEAD adgan/tap/contui`
Expected: Homebrew clones `https://github.com/adgan/contui.git` successfully. It may print `HEAD is now at …`; it must not 404.

- [ ] **Step 8: Confirm the install failure is Cargo's, not the formula's**

Run: `brew install --HEAD adgan/tap/contui`
Expected: FAIL, and the first error is Cargo's (no `Cargo.toml` in the repo yet). No `No such file or directory - assets/…` and no Ruby exception from the formula. Record the exact error line in the tap README's status note.

---

### Task 3: Audit workflow

**Files:**
- Create in `adgan/homebrew-tap`: `.github/workflows/audit.yml`

**Interfaces:**
- Consumes: `Formula/contui.rb` from Task 2.
- Produces: a GitHub Actions check named `audit`.

- [ ] **Step 1: Write the workflow**

```yaml
name: audit

on:
  push:
  pull_request:

permissions:
  contents: read

jobs:
  audit:
    runs-on: macos-15
    steps:
      - uses: actions/checkout@v4
      - name: Tap this checkout
        run: brew tap adgan/tap "$GITHUB_WORKSPACE"
      - name: Trust the formula
        run: brew trust adgan/tap
      - name: Audit
        run: brew audit --strict --online adgan/tap/contui
      - name: Style
        run: brew style
```

- [ ] **Step 2: Validate the YAML locally**

Run: `ruby -ryaml -e 'YAML.load_file(".github/workflows/audit.yml"); puts "ok"'`
Expected: `ok`.

- [ ] **Step 3: Run the same commands locally against the tapped repo**

```bash
brew tap adgan/tap 2>/dev/null || brew tap adgan/tap "$PWD"
brew trust adgan/tap
brew audit --strict --online adgan/tap/contui
brew style
```

Expected: audit silent apart from the rubocop-md warning; style `no offenses detected`. `--online` must not error on the `homepage`/`head` URLs.

- [ ] **Step 4: Commit, push and watch the first run**

```bash
git add .github/workflows/audit.yml
git commit -m "ci: audit and style the formula"
git push
gh run list -R adgan/homebrew-tap --limit 3
gh run view -R adgan/homebrew-tap --log-failed
```

Expected: one `audit` run, conclusion `success`. If it fails, fix the formula or the workflow, amend, and push again.

---

### Task 4: Tap README and the parent repo's follow-up task

**Files:**
- Modify in `adgan/homebrew-tap`: `README.md`
- Modify: `docs/superpowers/plans/2026-11-14-contui.md` (Task 9, Step 4; append Task 10)
- Modify: `docs/superpowers/specs/2026-11-14-docker-gpui-client-design.md` (append to "Deliverable and packaging")

**Interfaces:**
- Consumes: everything above.
- Produces: the documented install path `brew install --HEAD adgan/tap/contui`; parent-plan Task 10 defining v0.1.0, bottles and the cask.

- [ ] **Step 1: Write the full tap README**

Content, in this order: one-paragraph what-it-is; **Install** (`brew tap adgan/tap`, `brew trust adgan/tap`, `brew install --HEAD adgan/tap/contui`); **Launch** (`ln -sf "$(brew --prefix contui)/contUI.app" /Applications/contUI.app`, then right-click → Open on first launch); **Upgrade** (`brew update && brew reinstall --HEAD adgan/tap/contui`); **Uninstall** (`brew uninstall --force contui`, `brew untap adgan/tap`, remove the `/Applications` symlink); **Status** (HEAD-only until `v0.1.0`, needs Rust + Xcode Command Line Tools, no `brew install` yet); **Releasing** (checklist: tag `v0.1.0` in `adgan/contui`, add `url` + sha256, run `brew install --build-from-source --keep-temporary`, `brew bottle`, upload, merge bottle block, add cask).

- [ ] **Step 2: Verify the README's commands against the tap**

Run, in order, and confirm each behaves as the README says: `brew tap adgan/tap` (already tapped → no error), `brew info adgan/tap/contui`, `brew uninstall --force contui` (not installed → no error). Do not run a full install.
Expected: `brew info` shows version `0.1.0`, HEAD `main`, and the two caveat lines.

- [ ] **Step 3: Commit and push the tap**

```bash
git add README.md
git commit -m "docs: install, upgrade and release instructions"
git push
```

- [ ] **Step 4: Append Task 10 to the parent plan**

Task title `### Task 10: Homebrew release — stable url, bottle, cask`, placed after Task 9. Steps: tag `v0.1.0`; replace `version` with `url ".../archive/refs/tags/v0.1.0.tar.gz"` + sha256; verify `brew install adgan/tap/contui`; build and upload an arm64 bottle; add the bottle block; add `Casks/contui.rb` installing the prebuilt `contUI.app`; update README install line to `brew install --cask adgan/tap/contui`; acceptance: a machine without Rust installs and launches.

- [ ] **Step 5: Add the README line to Task 9**

In Task 9 Step 4 ("Write `README.md`"), append to the listed sections: `Homebrew: brew install --HEAD adgan/tap/contui (tap README covers upgrade and uninstall)`.

- [ ] **Step 6: Amend the parent design's packaging section**

Append a dated paragraph to "Deliverable and packaging": v1 still ships `just bundle`; distribution is the public tap `adgan/homebrew-tap`, HEAD-only until `v0.1.0`, with bottles and a cask deferred to plan Task 10. Link the tap design doc.

- [ ] **Step 7: Commit in the app repo**

```bash
git add docs/superpowers
git commit -m "docs: Homebrew distribution task and README hook"
```

- [ ] **Step 8: Final verification**

Run:
```bash
brew info adgan/tap/contui
gh repo view adgan/homebrew-tap --json nameWithOwner,visibility
gh run list -R adgan/homebrew-tap --limit 1
```
Expected: formula resolves with the caveats; repo public; latest audit run `success`.
