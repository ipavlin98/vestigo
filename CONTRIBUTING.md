# Contributing to Vestigo

Thanks for taking the time. Bug reports, ideas and pull requests are all welcome.

For anything bigger than a small fix, open an issue first and describe what you want to change. It saves you from
writing code that doesn't fit the project.

## Setup

You need Windows 10 or 11, [Rust](https://rustup.rs/) (stable) and the Visual Studio C++ build tools.

```
git clone https://github.com/ipavlin98/vestigo
cd vestigo
cargo build
```

The debug build is `target\debug\vestigo.exe`. It asks for administrator rights at start (it reads the NTFS file
table); decline and it still runs, with a slower index. Only one copy runs at a time: `vestigo.exe --quit` closes the
running one, which you have to do before rebuilding, because Windows locks the exe.

## Before you open a pull request

```
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
```

All three must pass; CI runs the same on x64 and also runs the tests on x86 and ARM64.

## Code rules

[AGENTS.md](AGENTS.md) is the full description of the architecture and the rules, written for people and coding
agents alike. Read the parts your change touches. The short version:

- Speed is the product. A change that slows down indexing or search is a regression, even if it adds a feature.
- No flicker. Every frame shows a complete state.
- Layers go one way: `app → launcher, app_window → ui → index, search, settings, platform`. Win32 and `unsafe` live
  only in `src/platform`, `src/index/ntfs.rs` and `src/main.rs`.
- Sizes come from `src/ui/tokens.rs`, colors from the theme, controls from `src/ui/widgets.rs`. Don't hand-roll them.
- No `unwrap`, `expect` or `panic!` outside tests. Clippy enforces most of this.
- Write a test when it can catch a real bug, not for the count.
- Keep it small. No abstractions or settings "for later".

## Pull requests

- One change per pull request, on a branch named `feat/…`, `fix/…`, `chore/…` or `docs/…`.
- The title becomes the commit on `main` (pull requests are squashed), so write it as a sentence about the change:
  "Preview: pictures load in the background".
- If users will notice the change, add a file to `changes/` — [changes/README.md](changes/README.md) explains the
  format in a few lines. Don't edit `CHANGELOG.md` or the version yourself.
- If the change touches something the READMEs describe, update both `README.md` and `README.ru.md`.
- Show visible changes with a screenshot or a short video.

## Translations

Every string the user sees lives in `src/i18n`: the list of fields in `i18n.rs`, the values in `i18n/en.rs` and
`i18n/ru.rs`. A new string goes into both languages. To add a language, copy `en.rs`, translate it and register it in
`i18n.rs`.

## Releasing

For maintainers.

```
scripts\release.ps1
```

The script folds the files from `changes/` into `CHANGELOG.md`, raises the version (minor if any file says `minor`,
otherwise patch) and opens a pull request that merges itself once CI passes. When that lands on `main`, the *Release*
workflow builds x64, x86 and ARM64, packs the installers and portable zips and publishes the GitHub release with the
changelog section and the list of merged pull requests. `scripts\release.ps1 -Local` only edits the files, so you can
look at the result first.

The workflow can also be started by hand (*Actions → Release → Run workflow*): it publishes the version that is in
`Cargo.toml` now. That is how the very first release is made.

## License

By contributing you agree that your work is released under the [MIT license](LICENSE).
