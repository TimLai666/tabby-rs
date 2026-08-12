import 'zone.js'
import 'core-js/proposals/reflect-metadata'

import './global.scss'
import './toastr.scss'

import * as AngularAnimations from '@angular/animations'
import * as AngularCdkClipboard from '@angular/cdk/clipboard'
import * as AngularCdkDragDrop from '@angular/cdk/drag-drop'
import * as AngularCommon from '@angular/common'
import * as AngularCompiler from '@angular/compiler'
import * as AngularCore from '@angular/core'
import * as AngularForms from '@angular/forms'
import * as AngularPlatformBrowser from '@angular/platform-browser'
import * as AngularPlatformBrowserAnimations from '@angular/platform-browser/animations'
import * as AngularPlatformBrowserDynamic from '@angular/platform-browser-dynamic'
import * as NgBootstrap from '@ng-bootstrap/ng-bootstrap'
import * as NgxToastr from 'ngx-toastr'
import * as RxJS from 'rxjs'
import * as RxJSOperators from 'rxjs/operators'

import * as CoreExports from '../../tabby-core/src'
import LocalModule, * as LocalExports from '../../tabby-local/src'
import PluginManagerModule from '../../tabby-plugin-manager/src'
import SettingsModule, * as SettingsExports from '../../tabby-settings/src'
import * as TauriExports from '../../tabby-tauri/src'
import * as TerminalExports from '../../tabby-terminal/src'
import { bootstrapTabby } from './bootstrap'
import {
    loadPluginModules,
    PluginModuleRegistry,
} from './plugin-runtime/runtime'

const CoreModule = CoreExports.default
const CoreBootstrap = CoreExports.bootstrap
const TauriModule = TauriExports.default
const TAURI_RUNTIME_INFO = TauriExports.TAURI_RUNTIME_INFO
const TauriHostBridge = TauriExports.TauriHostBridge

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

function registerPackage (registry: PluginModuleRegistry, name: string, value: unknown): void {
    registry.register(name, value)
    if (name.startsWith('tabby-')) {
        registry.register(`terminus-${name.substring('tabby-'.length)}`, value)
    }
}

function createPluginRegistry (): PluginModuleRegistry {
    const registry = new PluginModuleRegistry()
    registerPackage(registry, 'tabby-core', CoreExports)
    registerPackage(registry, 'tabby-local', LocalExports)
    registerPackage(registry, 'tabby-settings', SettingsExports)
    registerPackage(registry, 'tabby-tauri', TauriExports)
    registerPackage(registry, 'tabby-terminal', TerminalExports)
    registry.register('@angular/animations', AngularAnimations)
    registry.register('@angular/cdk/clipboard', AngularCdkClipboard)
    registry.register('@angular/cdk/drag-drop', AngularCdkDragDrop)
    registry.register('@angular/common', AngularCommon)
    registry.register('@angular/compiler', AngularCompiler)
    registry.register('@angular/core', AngularCore)
    registry.register('@angular/forms', AngularForms)
    registry.register('@angular/platform-browser', AngularPlatformBrowser)
    registry.register('@angular/platform-browser/animations', AngularPlatformBrowserAnimations)
    registry.register('@angular/platform-browser-dynamic', AngularPlatformBrowserDynamic)
    registry.register('@ng-bootstrap/ng-bootstrap', NgBootstrap)
    registry.register('ngx-toastr', NgxToastr)
    registry.register('rxjs', RxJS)
    registry.register('rxjs/operators', RxJSOperators)
    registry.register('zone.js', {})
    registry.register('zone.js/dist/zone.js', {})
    return registry
}

function pluginBlacklist (config: Record<string, unknown>): string[] {
    return Array.isArray(config.pluginBlacklist)
        ? config.pluginBlacklist.filter((value): value is string => typeof value === 'string')
        : []
}

function percentile (values: number[], fraction: number): number {
    const sorted = [...values].sort((left, right) => left - right)
    return sorted[Math.min(sorted.length - 1, Math.max(0, Math.ceil(sorted.length * fraction) - 1))]
}

async function reportBenchmarkFrames (bridge: InstanceType<typeof TauriHostBridge>): Promise<void> {
    const frameTimes: number[] = []
    let previous = performance.now()
    let droppedFrameCount = 0
    const expectedFrameTime = 1000 / 60
    await new Promise<void>(resolve => {
        const capture = (timestamp: number): void => {
            const frameTime = timestamp - previous
            previous = timestamp
            frameTimes.push(frameTime)
            droppedFrameCount += Math.max(0, Math.round(frameTime / expectedFrameTime) - 1)
            if (frameTimes.length >= 120) {
                resolve()
            } else {
                window.requestAnimationFrame(capture)
            }
        }
        window.requestAnimationFrame(capture)
    })
    await bridge.invoke('app.benchmarkFrameReport', {
        method: 'requestAnimationFrame trace',
        samples: frameTimes.length,
        p95FrameTimeMs: percentile(frameTimes, 0.95),
        droppedFrameCount,
    })
}

async function main (): Promise<void> {
    const bridge = new TauriHostBridge()
    window['retryPluginBootstrap'] = async () => {
        await bridge.invoke('plugins.bootstrapRetry', {})
        window.location.reload()
    }
    updateProgress(10)

    const [bootstrapData, runtimeInfo] = await Promise.all([
        bridge.invoke('app.bootstrap', {}),
        bridge.invoke('app.runtimeInfo', {}),
    ])

    window['__TABBY_PLATFORM__'] = runtimeInfo.platform
    window['__TABBY_ARCH__'] = runtimeInfo.arch
    if (runtimeInfo.benchmarkReadyFile) {
        window.addEventListener('tabby:terminal-ready', () => {
            void (async () => {
                if (runtimeInfo.benchmarkFrameReportFile) {
                    await reportBenchmarkFrames(bridge)
                }
                await bridge.invoke('app.benchmarkReady', {})
            })()
        }, { once: true })
    }
    updateProgress(40)

    const pluginResult = await loadPluginModules({
        discover: bootstrapData.safeMode
            ? async () => []
            : () => bridge.invoke('plugins.discover', {}),
        readEntry: packageName => bridge.invoke('plugins.readEntry', { packageName }),
    }, createPluginRegistry(), bootstrapData.safeMode ? [] : pluginBlacklist(bootstrapData.config), {
        pluginStarted: plugin => bridge.invoke('plugins.bootstrapPluginStarted', {
            packageName: plugin.packageName,
        }),
        pluginCompleted: plugin => bridge.invoke('plugins.bootstrapPluginCompleted', {
            packageName: plugin.packageName,
        }),
    })
    ;(window as any).pluginLoadFailures = pluginResult.failures
    const discoveryFailure = pluginResult.failures.find(failure => failure.phase === 'discover')
    if (discoveryFailure) {
        window['safeModeReason'] = discoveryFailure.message
        await bridge.invoke('plugins.bootstrapFailed', {
            packageName: null,
            phase: 'discover',
            message: discoveryFailure.message,
        }).catch(journalError => console.warn('Could not journal plugin discovery failure:', journalError))
    }
    for (const failure of pluginResult.failures) {
        if (failure.phase === 'discover') continue
        await bridge.invoke('plugins.bootstrapFailed', {
            packageName: failure.plugin.packageName,
            phase: failure.phase,
            message: failure.message,
        }).catch(journalError => console.warn(`Could not journal plugin failure for ${failure.plugin.packageName}:`, journalError))
    }
    if (bootstrapData.safeMode && bootstrapData.safeModeReason) {
        window['safeModeReason'] = bootstrapData.safeModeReason
    }
    window['safeModeSuspectedPlugins'] = bootstrapData.safeModeSuspectedPlugins ?? []
    updateProgress(60)

    const coreModule = CoreModule.forRoot() as any
    coreModule.ngModule.pluginName = 'core'
    coreModule.bootstrap = CoreBootstrap

    const settingsModule = SettingsModule as any
    settingsModule.pluginName = 'settings'

    const localModule = LocalModule as any
    localModule.pluginName = 'local'

    const pluginManagerModule = PluginManagerModule as any
    pluginManagerModule.pluginName = 'plugin-manager'

    const tauriModule = TauriModule as any
    tauriModule.pluginName = 'tauri'

    const pluginModules = [
        coreModule,
        settingsModule,
        localModule,
        tauriModule,
        pluginManagerModule,
        ...pluginResult.modules,
    ]
    const bootstrapOptions = {
        debug: false,
        extraProviders: [
            { provide: TAURI_RUNTIME_INFO, useValue: runtimeInfo },
        ],
    }

    let fallbackUsed = false
    try {
        await bootstrapTabby(bootstrapData, pluginModules, bootstrapOptions)
    } catch (error) {
        await bridge.invoke('plugins.bootstrapFailed', {
            packageName: null,
            phase: 'angular-bootstrap',
            message: String(error),
        }).catch(journalError => console.warn('Could not journal Angular bootstrap failure:', journalError))
        if (bootstrapData.safeMode) {
            throw error
        }
        window['safeModeReason'] = error
        fallbackUsed = true
        await bootstrapTabby(bootstrapData, [
            coreModule,
            settingsModule,
            localModule,
            tauriModule,
            pluginManagerModule,
        ], bootstrapOptions)
    }

    if (!fallbackUsed && pluginResult.failures.length === 0) {
        await bridge.invoke('plugins.bootstrapSucceeded', {})
    }

    updateProgress(100)
}

void main().catch(showBootstrapError)
