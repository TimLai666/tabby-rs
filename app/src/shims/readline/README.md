# Readline in the Tauri renderer

`TerminalStreamProcessor` and `cli-spinner` need Node's stream-based line editor
and ANSI cursor functions. Tauri runs them in a WebView without Node built-ins.
The parent `readline.cjs` exports the APIs used by these consumers.

The four editing modules and ANSI pattern come from Node 22.22.0 embedded in
Electron 38.8.6. `provenance.json` records the original source hashes. Each copied
module includes Node's MIT notice, also available in `LICENSE`. The source
transformation changes imports and supplies browser timers and helper functions.
The line editing algorithms remain unchanged.

`runtime.cjs` adapts the required internal helpers. `widths.json` contains all
non-default Unicode scalar widths from the reference runtime's ICU 74.2, Unicode
15.1. Using these ranges avoids relying on the WebView's Unicode version for
cursor placement. Width calculation preserves Node's ASCII prefix and NFC rules.
The derived ICU/Unicode data retains the Unicode License V3 in `UNICODE-LICENSE`
and in the runtime module's bundled notice.

Regenerate with the pinned Electron executable, from the repository root:

```sh
ELECTRON_RUN_AS_NODE=1 node_modules/electron/dist/Electron.app/Contents/MacOS/Electron scripts/vendor-readline.cjs
```

Run `node scripts/test-tauri-readline.mjs` to test the actual browser bundle
resolution, spinner, editing bytes, history, and cleanup against Node readline.
The test launches the pinned Electron executable automatically and also checks
every code point's width against that runtime's ICU implementation.

This adapter does not implement the entire Node readline public API. In particular,
Linux process suspension via Ctrl+Z and the inherited async iterator require platform
integration. A macOS Electron GUI renderer probe with the original window settings
confirms SIGTSTP is ignored in that launch environment and input remains usable.
The adapter preserves that result. Windows retains Node's Ctrl+Z no-op branch.
The current consumers use line events, prompt, close, and cursor
functions. Desktop connector acceptance and other platform behavior must be
verified independently of the bundle tests.
