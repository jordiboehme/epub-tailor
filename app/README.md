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
