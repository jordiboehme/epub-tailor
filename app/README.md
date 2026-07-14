# EPUB Tailor - desktop app

A Tauri v2 shell around the `epub-tailor` CLI. The Rust crate (`src-tauri/`)
joins the cargo workspace; the frontend is Svelte 5 + TypeScript + Tailwind v4,
built by Vite. The CLI ships inside the app as a bundled sidecar and every
feature drives it through its `--report json` interface.

## Development

```sh
cd app
npm install
npm run tauri dev
```

`tauri dev` runs `prepare-sidecar` first: it builds `epub-tailor` in release mode
and drops it into `src-tauri/binaries/epub-tailor-<target-triple>` so the sidecar
resolves. `npm run build` produces the frontend in `dist/`, and `npm run check`
type-checks it with svelte-check.

## The dist gotcha

`tauri::generate_context!` embeds `../dist` at compile time, so the frontend must
be built before anything touches the Rust crate. Run `npm run build` before
`cargo clippy -p epub-tailor-app` or `cargo build` on the app, or the macro fails
with a missing-`dist` error. `npm run tauri dev` and `npm run tauri build` handle
this for you via `beforeDevCommand`/`beforeBuildCommand`.

The sidecar is the same kind of gotcha: `tauri-build` resolves
`binaries/epub-tailor-<target-triple>` at build-script time and fails with
`resource path ... doesn't exist` if it is not there. `npm run prepare-sidecar`
puts it there.

## Bundling gotcha: the updater key

`bundle.createUpdaterArtifacts` is on, so `npm run tauri build` signs the update
artifact it produces (`.app.tar.gz`, the NSIS installer, the AppImage) and
*fails* without a private key rather than shipping something the app would
refuse to install:

```
A public key has been found, but no private key.
Make sure to set `TAURI_SIGNING_PRIVATE_KEY` environment variable.
```

Any key satisfies it locally - nothing built on your machine is ever installed by
anyone's updater:

```sh
npx tauri signer generate -w /tmp/throwaway.key
export TAURI_SIGNING_PRIVATE_KEY="$(cat /tmp/throwaway.key)"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=''
npm run tauri build -- --bundles app,dmg
```

`npm run tauri dev` does not bundle, so it needs none of this. The real key, the
release flow and the secrets CI needs are documented in
[`docs/releasing.md`](../docs/releasing.md).
