# Some background

Tabby RS is a Tauri desktop application with an Angular/TypeScript frontend and
a Rust host. The production desktop path is built with the Tauri CLI; the
Electron entry and `tabby-electron` package remain only as legacy comparison
code while the parity gate is still open.

# Getting started

First of all, clone this repository.

# Install Dependencies
- [Node.js](https://nodejs.org/en/download/) **version 22**
- [Yarn](https://yarnpkg.com/)
- Rust stable and the platform dependencies listed in [CONTRIBUTING.md](CONTRIBUTING.md)

From the repository root, install the dependencies via yarn:

```sh
yarn install --frozen-lockfile
```

For Linux package dependencies, use the current list in [CONTRIBUTING.md](CONTRIBUTING.md) rather than copying an old distribution-specific command.

Build Tabby:

```sh
yarn build:tauri:frontend
cargo build --manifest-path src-tauri/Cargo.toml
```

Start the Tauri desktop application:

```sh
yarn start:tauri
```

# Building a Tauri installer

The release workflow builds the frontend and then invokes the pinned Tauri CLI. For a local bundle, install the matching CLI and run the target command for your platform:

```sh
yarn build:tauri:frontend
cargo tauri build --bundles app --no-sign \
  --config '{"bundle":{"active":true,"targets":["app"],"createUpdaterArtifacts":false}}'
```

The release matrix uses `nsis` on Windows, `dmg` on macOS, and `appimage,deb,rpm` on Linux. See [.github/workflows/release.yml](.github/workflows/release.yml) for the authoritative target and signing configuration.
CI generates `src-tauri/tauri.release.conf.json` from release secrets before a signed build; that generated file is intentionally not committed.

The `scripts/build-windows.mjs`, `scripts/build-macos.mjs`, and
`scripts/build-linux.mjs` scripts are legacy Electron packaging helpers. They
are not the release path and must not be used to validate a Tauri bundle.

Tauri artifacts are produced under `src-tauri/target/release/bundle/`.

# Project layout
```
tabby
├─ app                                  # Angular renderer and Tauri frontend build
|  ├─ src                               # Shared renderer source
|  └─ dist-tauri                        # Generated Tauri frontend output
├─ src-tauri                            # Rust host, commands, state and bundling
├─ tabby-tauri                          # Tauri bridge/provider implementation
├─ build
├─ clink                                # Clink distribution, for Windows
├─ scripts                              # Maintenance scripts
├─ tabby-community-color-schemes     # Plugin that provides color schemes
├─ tabby-core                        # Plugin that provides base UI and tab management
├─ tabby-local                       # Plugin that provides local shells and profiles
├─ tabby-plugin-manager              # Plugin that installs other plugins
├─ tabby-settings                    # Plugin that provides the settings tab
├─ tabby-terminal                    # Plugin that provides terminal tabs
├─ tabby-web                         # Plugin that provides web-specific functions
└─ tabby-electron                    # Legacy Electron comparison code
```

# Plugin layout
```
tabby-pluginname
├─ src                                  # Typescript code
|  ├─ components                        # Angular components
|  |  ├─ foo.component.ts               # Code
|  |  ├─ foo.component.scss             # Styles
|  |  └─ foo.component.pug              # Template
|  ├─ services                          # Angular services
|  |  └─ foo.service.ts
|  ├─ api.ts                            # Publicly exported API
|  └─ index.ts                          # Module entry point
├─ package.json
├─ tsconfig.json
└─ webpack.config.js
```

# Plugins

The app will load all plugins from the source checkout in the dev mode, from the user's plugins directory at all times (click `Open Plugins Directory` under `Settings` > `Plugins`) and from the directory specified by the `TABBY_PLUGINS` environment var.

Only modules whose `package.json` file contains a `tabby-plugin` keyword will be loaded.

If you're currently in your plugin's directory, start Tabby as `TABBY_PLUGINS=$(pwd) tabby --debug`

A plugin should only provide a default export, which should be a `NgModule` class (or a `NgModuleWithDependencies` where applicable). This module will be injected as a dependency to the app's root module.

```javascript
import { NgModule } from '@angular/core'

@NgModule()
export default class MyModule {
  constructor () {
    console.log('Angular engaged, cap\'n.')
  }
}
```

Plugins provide functionality by exporting singular or multi providers:


```javascript
import { NgModule, Injectable } from '@angular/core'
import { ToolbarButtonProvider, ToolbarButton } from 'tabby-core'

@Injectable()
export class MyButtonProvider extends ToolbarButtonProvider {
    provide (): ToolbarButton[] {
        return [{
            icon: 'star',
            title: 'Foobar',
            weight: 10,
            click: () => {
                alert('Woohoo!')
            }
        }]
    }
}

@NgModule({
    providers: [
        { provide: ToolbarButtonProvider, useClass: MyButtonProvider, multi: true },
    ],
})
export default class MyModule { }
```


See `tabby-core/src/api.ts`, `tabby-settings/src/api.ts`, `tabby-local/src/api.ts` and `tabby-terminal/src/api.ts` for the available extension points.

Also check out [the example plugin](https://github.com/Eugeny/tabby-clippy).

Publish your plugin on NPM with a `tabby-plugin` keyword to make it appear in the Plugin Manager.
