import 'zone.js'
import 'core-js/proposals/reflect-metadata'
import 'rxjs'

import './global.scss'
import './toastr.scss'

import CoreModule, { bootstrap as CoreBootstrap } from '../../tabby-core/src'
import TauriModule, {
    TAURI_RUNTIME_INFO,
    TauriHostBridge,
} from '../../tabby-tauri/src'
import { bootstrapTabby } from './bootstrap'

location.hash = ''

function updateProgress (percent: number): void {
    const progressBar = document.querySelector<HTMLElement>('.progress .bar')
    if (progressBar) {
        progressBar.style.width = `${percent}%`
    }
}

function showBootstrapError (error: unknown): void {
    console.error('Tauri Angular bootstrapping error:', error)
    const root = document.querySelector('app-root')
    if (root) {
        root.textContent = `Tabby RS failed to start: ${String(error)}`
    }
}

async function main (): Promise<void> {
    const bridge = new TauriHostBridge()
    updateProgress(10)

    const [bootstrapData, runtimeInfo] = await Promise.all([
        bridge.invoke('app.bootstrap', {}),
        bridge.invoke('app.runtimeInfo', {}),
    ])

    window['__TABBY_PLATFORM__'] = runtimeInfo.platform
    window['__TABBY_ARCH__'] = runtimeInfo.arch
    updateProgress(40)

    const coreModule = CoreModule.forRoot() as any
    coreModule.ngModule.pluginName = 'core'
    coreModule.bootstrap = CoreBootstrap

    const tauriModule = TauriModule as any
    tauriModule.pluginName = 'tauri'

    await bootstrapTabby(bootstrapData, [coreModule, tauriModule], {
        debug: false,
        extraProviders: [
            { provide: TAURI_RUNTIME_INFO, useValue: runtimeInfo },
        ],
    })

    updateProgress(100)
}

void main().catch(showBootstrapError)
