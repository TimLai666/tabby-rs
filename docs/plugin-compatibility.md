# Plugin compatibility

Tabby RS supports the defined npm plugin workflow, not every plugin that happens to install in upstream Tabby.

## Supported shape

A plugin should use the public Tabby plugin API, avoid Electron imports and private host internals, and avoid native Node addons. The package must be discoverable by the supported Tabby plugin keywords or an existing legacy plugin alias.

Node.js and npm are used only when the user explicitly searches for, installs, updates, or removes a plugin. The desktop application remains usable without Node.js. When Node.js or npm cannot be verified, plugin management is disabled with the detected reason rather than silently running a fallback command.

## Failure and safe mode

Install and update commands use argument arrays, isolated plugin directories, bounded output, cancellation, and redacted diagnostics. A failed plugin load is isolated from the rest of the runtime. Safe mode can retry, disable, or remove the failing plugin without copying executable plugin code into the application bundle.

The official fixture and the repository's supported compatibility tests are the release evidence. A third-party plugin may still require adaptation if it depends on Electron, private APIs, a native addon, or an unsupported host assumption.

## Runtime security boundary

The Tauri renderer CSP includes `unsafe-eval` because the compatibility layer evaluates existing CommonJS plugin bundles with `new Function`. Tauri CSP cannot scope that directive to only the local plugin evaluator, so this is a deliberate high-trust trade-off: plugin code runs with the renderer's privileges and the evaluator is not a sandbox.

The evaluator does not load remote scripts and does not fall back to the system Node.js module resolver. Plugin `require()` calls are limited to the registered Angular, RxJS, and public Tabby modules, while Electron, Node.js built-ins, and native addon requests are rejected with an explicit incompatibility error.
