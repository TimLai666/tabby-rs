import { ApplicationRef, enableProdMode, NgModuleRef, StaticProvider } from '@angular/core'
import { enableDebugTools } from '@angular/platform-browser'
import { platformBrowserDynamic } from '@angular/platform-browser-dynamic'

import { BOOTSTRAP_DATA, BootstrapData } from '../../tabby-core/src/api/mainProcess'
import { getRootModule } from './app.module'

export interface BootstrapOptions {
    debug?: boolean
    extraProviders?: StaticProvider[]
}

let productionModeEnabled = false

/**
 * Boots the shared Angular application from a host-provided module list.
 * Electron and Tauri entries both call this function; host-specific loading
 * and IPC remain outside of the Angular root module.
 */
export async function bootstrapTabby (
    bootstrapData: BootstrapData,
    pluginModules: any[],
    options: BootstrapOptions = {},
): Promise<NgModuleRef<any>> {
    if (!options.debug && !productionModeEnabled) {
        enableProdMode()
        productionModeEnabled = true
    }

    window['pluginModules'] = pluginModules

    const rootModule = getRootModule(pluginModules)
    const extraProviders = options.extraProviders ?? []
    const moduleRef = await platformBrowserDynamic([
        { provide: BOOTSTRAP_DATA, useValue: bootstrapData },
        ...extraProviders,
    ]).bootstrapModule(rootModule)

    if (options.debug) {
        const applicationRef = moduleRef.injector.get(ApplicationRef)
        enableDebugTools(applicationRef.components[0])
    }

    return moduleRef
}
