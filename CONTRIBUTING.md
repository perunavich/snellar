# Contributing to Snellar

Bug reports, fixes and translations are welcome. For anything larger than a fix, open an issue first and say what you want to change, so we can agree on it before you spend the time.

This repository gets one commit per release from the development tree. A pull request accepted here is taken into that tree, goes through the full set of checks there, and shows up here with the next release rather than as its own commit.

## Build and run

On Debian 13 (other distributions need the same Qt 6.8 modules under their own names):

```sh
sudo apt install build-essential pkg-config clang libclang-dev cmake git mpv \
  qt6-base-dev qt6-base-dev-tools qt6-declarative-dev qt6-declarative-dev-tools \
  qt6-shadertools-dev qt6-shader-baker qt6-l10n-tools qt6-qmllint-plugins \
  qt6-svg-plugins qt6-image-formats-plugins qt6-qpa-plugins qt6-wayland libxkbcommon-dev \
  qml6-module-qtquick qml6-module-qtquick-controls qml6-module-qtquick-templates \
  qml6-module-qtquick-layouts qml6-module-qtquick-shapes qml6-module-qtquick-effects \
  qml6-module-qtquick-window qml6-module-qtquick-dialogs qml6-module-qtquick-particles \
  qml6-module-qtqml qml6-module-qtqml-models qml6-module-qtqml-workerscript qml6-module-qtcore \
  qml6-module-qt-labs-platform qml6-module-qt-labs-settings qml6-module-qt5compat-graphicaleffects
```

Install Rust with [rustup](https://rustup.rs); it picks up the version pinned in `rust-toolchain.toml`. For the two script checks below you also need [Bun](https://bun.sh).

Run a development build with its own data folders, so it never touches your real library and does not hand over to an installed Snellar that is already running:

```sh
dev=/tmp/snellar-dev
XDG_DATA_HOME=$dev/data XDG_CACHE_HOME=$dev/cache SNELLAR_SOCK=$dev/snellar.sock cargo run
```

Add `--lang=uk` (or `ru`, `en`) after `cargo run --` to start it in another language, and `--size=960x620` for a given window size.

## Before you open a pull request

Run what this repository can check:

```sh
cargo fmt --check
cargo clippy --workspace --locked -- -D warnings
tools/qmllint.sh             # after a build; must print no warnings
bun tools/i18n-check.ts      # every qsTr string has en, ru and uk text
```

Then open the app and look at what you changed, in a small window (960 × 620) and a large one, in English, Russian and Ukrainian.

The tests, their fixtures and fakes, and the UI scenarios that run under Xvfb are not published, and the `#[cfg(test)]` code is cut out of the published source, so `cargo test` has nothing to run here. They run on your change before it is merged. A bug fix should still come with the test you would expect to fail before the fix, in a `#[cfg(test)] mod tests` at the end of the file; if you can't write one, describe in the pull request how to reproduce the bug.

## Code

- **No comments.** Names and small functions carry the meaning. The one exception is `// SAFETY:` where Rust requires it.
- **Match the file you are in.** Don't reformat code you are not changing. `rustfmt.toml` sets the width to 140.
- **Lints are fixed, not silenced.** `#[expect(lint, reason = "...")]` only where clippy is wrong, with the reason. `clippy.toml` bans a few calls on purpose, such as blocking `Command::output` in async code.
- **Four crates, one direction.** The `snellar` binary depends on `snellar-app`, which depends on `snellar-youtube`, which depends on `snellar-core`. Only the binary pulls Qt; D-Bus, MCP and Discord live only in `snellar-app`; `snellar-core` makes no network requests.
- **Errors carry a code.** A failure gets a `Code` from `crates/core/src/error.rs`. Never decide what went wrong by reading an error message. A new kind of failure gets a new code; a code never changes meaning.
- **Rust writes no text the window shows.** It sends codes and data; QML turns them into words (`qml/Errors.qml` for errors).
- **Track ids** are `TrackId` (`yt:<videoId>`). Don't slice, prefix or check an id string by hand.
- **Settings** are rows in `settings::ALL` in `crates/core/src/settings.rs`.
- **Library migrations are append-only.** Never edit a step that has shipped; a schema change is a new step in `crates/core/src/library/migrations.rs`.
- **Background work** in the player and recommendations is spawned through a `TaskSet`. After an `await` whose result is written into player state, check that the user has not moved on in the meantime (`Player::superseded`).
- **The API is generated documentation.** After changing `api::METHODS`, a setting or an error code, regenerate the reference: `cargo run -- ctl --docs > docs/api.md`.
- **Scripts** are Bash, or TypeScript run with Bun. No Python.

## Interface

- Colours, sizes, radii and durations come from `qml/Theme.qml`; spring numbers come from `qml/morph/Motion.qml`. No numbers of your own in a view.
- Every visible string goes through `qsTr()` and gets a translation in `i18n/snellar_en.ts`, `snellar_ru.ts` and `snellar_uk.ts`. Use `%1` with `.arg()` instead of joining strings, and `%n` for plurals. To pick up new strings:

  ```sh
  lupdate qml -extensions qml -source-language en -locations none -ts i18n/snellar_en.ts i18n/snellar_uk.ts i18n/snellar_ru.ts
  ```

- Spacing is on a 4 px grid (`Theme.spaceXs` to `Theme.space2xl`). Controls in one row share one height.
- The accent colour means only three things: playing, switched on, and the one main action of a screen. Navigation and selection stay neutral.
- Text has at least 4.5:1 contrast against what is behind it. Anything clickable is at least 28 × 28 px.
- Every control has rest, hover, pressed, keyboard focus and disabled states. A state change never changes a control's size.
- Anything that moves uses a spring from `Motion`, and lands at once when reduced motion is on. Frequent actions (switching tracks, hovering a row, selecting) change colour only.
- When it's unclear how something should behave, do what Spotify and YouTube Music do.

## Commits and pull requests

- One change per commit. The message starts with the area and says what is true after the change, for example `queue: a removed track is not brought back by autoplay`.
- In the pull request, say what changed, why, and how you checked it. For a change on screen, add screenshots of the small and the large window.
- By contributing you agree that your work is licensed under [GPL-3.0-or-later](LICENSE), like the rest of Snellar.
