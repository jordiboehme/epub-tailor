# Releasing

Everything ships from one tag. `v0.4.2` on `main` builds the CLI for six targets and the desktop app for the same six, then signs and notarizes what Apple wants signed and notarized. It leaves a **draft** GitHub release for a human to read before the world does. Publishing that draft is what updates the Homebrew tap and what makes the app's auto-updater see the new version at all.

## The secrets a maintainer must set

All of these live in the repository's Actions secrets (Settings, Secrets and variables, Actions). Without them a tag build fails on purpose: a release that quietly skipped signing is worse than no release.

| Secret | What it is | Used by |
|---|---|---|
| `APPLE_CERTIFICATE_P12` | Base64 of the Developer ID Application certificate, exported as `.p12` | CLI signing, app signing |
| `APPLE_CERTIFICATE_PASSWORD` | The password that `.p12` was exported with | CLI signing, app signing |
| `APPLE_TEAM_ID` | The 10-character Apple team id | CLI signing, app signing |
| `APPLE_API_KEY_P8` | The App Store Connect API key, the whole `AuthKey_XXXX.p8` file | notarization |
| `APPLE_API_KEY_ID` | That key's id | notarization |
| `APPLE_API_ISSUER_ID` | The issuer id from App Store Connect | notarization |
| `TAURI_SIGNING_PRIVATE_KEY` | The updater's minisign private key, one line of base64 | app updater artifacts |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | That key's password | app updater artifacts |
| `TAP_GITHUB_TOKEN` | A token with write access to `jordiboehme/homebrew-tap` | formula and cask updates |

The Apple secrets are mapped onto the names Tauri's bundler expects (`APPLE_CERTIFICATE`, `APPLE_API_KEY`, `APPLE_API_ISSUER`, `APPLE_API_KEY_PATH`) inside `release.yml`. They predate the app, which is why the names do not line up.

## The updater keypair

The app checks every update it downloads against a public key compiled into it. That key is already in `app/src-tauri/tauri.conf.json` under `plugins.updater.pubkey`. Its private half signs the artifacts in CI, and it exists exactly once:

```sh
cd app
npx tauri signer generate -w epub-tailor-updater.key
```

That prints the public key and writes two files. The private key (`epub-tailor-updater.key`) goes into `TAURI_SIGNING_PRIVATE_KEY`, its password into `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` and then the file is deleted. `*.key` is gitignored, and it must stay out of the repo: anyone holding it can sign an update that every installed copy of the app will accept and run.

Losing it is worse than it sounds. A new keypair means a new public key, which means a new build, which the already-installed copies will refuse to install because it is signed by a key they never heard of. Everyone would have to reinstall by hand. Keep a copy somewhere safe.

Rotating the pubkey in `tauri.conf.json` and the two secrets together is fine, as long as you accept that this is what happens to the people running the old build.

## Releasing

1. Bump `version` under `[workspace.package]` in `Cargo.toml`, then `cargo check` so `Cargo.lock` follows. The app takes its version from there too, and a tag that disagrees with it fails the build on purpose: the updater compares `latest.json`'s version against the version the running app reports, so a mismatch would offer everyone an update to the release they are already running, forever. The check is an exact string match, not a semver-aware one, so a prerelease tag such as `v0.5.0-rc.1` needs `[workspace.package] version` set to the identical `0.5.0-rc.1`, or the build fails the same way.
2. Commit, tag `vX.Y.Z`, push the tag.
3. `release.yml` builds `build` (six CLI targets) and `build-app` (six app bundles), then `publish` collects everything, writes `SHA256SUMS` and `latest.json`, generates notes with git-cliff and creates a **draft** release.
4. Read the notes, fix them up, publish the release.
5. Publishing fires `homebrew.yml`, which regenerates both the CLI formula and the app cask in the tap and pushes them in one commit.

A `workflow_dispatch` run of `release.yml` is a dry run: it builds the whole matrix and stops before publishing. Without the Apple secrets it produces unsigned macOS bundles, and without the updater key it signs its updater artifacts with a throwaway key it generates on the spot, so the bundling path still gets exercised end to end.

## Why the draft is safe

The app polls `https://github.com/jordiboehme/epub-tailor/releases/latest/download/latest.json`, and `releases/latest` only ever resolves to a **published** release. A draft is invisible to it. So the assets can sit in the draft for as long as the notes need, and nobody is offered an update until you press the button.

The `publish` job hard-fails if any platform's updater artifact or its `.sig` is missing. That is deliberate: a `latest.json` with a hole in it means every user on that platform silently stops receiving updates, and a published release is not something you can take back.

## What a release contains

Per platform, for the CLI: `epub-tailor-vX.Y.Z-<platform>.tar.gz` (or `.zip` on Windows). For the app:

| Platform | Installer | Updater artifact |
|---|---|---|
| macos-arm64, macos-intel | `EPUB-Tailor-vX.Y.Z-<platform>.dmg` | `.app.tar.gz` and `.app.tar.gz.sig` |
| windows-amd64, windows-arm64 | `EPUB-Tailor-vX.Y.Z-<platform>-setup.exe` | the same `.exe`, and its `.sig` |
| linux-amd64, linux-arm64 | `.AppImage` and `.deb` | the same `.AppImage`, and its `.sig` |

Plus `SHA256SUMS` (the cask reads the DMG hashes out of it) and `latest.json`.

The Windows installers are unsigned: a code-signing certificate costs real money, and SmartScreen's warning is the price of not paying it. macOS is signed with a Developer ID and notarized, and the ticket is stapled to the `.app` and to the DMG, so a first launch works offline and without a right-click-Open dance.

## Building the app locally

`npm run tauri build` in `app/` now needs the updater key in the environment, because `bundle.createUpdaterArtifacts` is on and Tauri refuses to build an unsigned update artifact for a key it has a public half of:

```sh
export TAURI_SIGNING_PRIVATE_KEY="$(cat /path/to/epub-tailor-updater.key)"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD='...'
npm run tauri build -- --bundles app,dmg
```

Any key works for a local build - nothing you build locally is going to be installed by anyone's updater. Generate a throwaway one with `npx tauri signer generate -w /tmp/throwaway.key` if you do not have the real one to hand. `npm run tauri dev` does not bundle, so it needs none of this.
