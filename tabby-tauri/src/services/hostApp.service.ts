import { Inject, Injectable, Injector, NgZone } from '@angular/core'
import { CLIEvent, CLIHandler, HostAppService, Platform } from 'tabby-core'

import { HostBridge, LaunchContext, RuntimeInfo, TAURI_RUNTIME_INFO } from '../api/hostBridge'

function mapPlatform (platform: string): Platform {
    switch (platform.toLowerCase()) {
        case 'windows':
        case 'win32':
            return Platform.Windows
        case 'macos':
        case 'darwin':
            return Platform.macOS
        case 'linux':
            return Platform.Linux
        default:
            return Platform.Web
    }
}

@Injectable()
export class TauriHostAppService extends HostAppService {
    readonly platform: Platform
    readonly configPlatform: Platform
    readonly windowsBuild: number|undefined

    private ready = false
    private pendingLaunches: LaunchContext[] = []

    constructor (
        private injector: Injector,
        private zone: NgZone,
        private bridge: HostBridge,
        @Inject(TAURI_RUNTIME_INFO) runtimeInfo: RuntimeInfo,
    ) {
        super(injector)
        this.platform = mapPlatform(runtimeInfo.platform)
        this.configPlatform = this.platform
        this.windowsBuild = runtimeInfo.windowsBuild ?? undefined

        void this.bridge.listen('app:launch', context => this.enqueueLaunch(context)).catch(error => {
            this.logger.error('Failed to listen for launch requests:', error)
        })
        void this.bridge.invoke('app.initialLaunch', {}).then(context => {
            if (context) {
                this.enqueueLaunch(context)
            }
        }).catch(error => {
            this.logger.error('Failed to read the initial launch request:', error)
        })
    }

    newWindow (): void {
        void this.bridge.invoke('window.new', {}).catch(error => {
            this.logger.warn('Failed to open a new window:', error)
        })
    }

    emitReady (): void {
        this.ready = true
        const pending = this.pendingLaunches.splice(0)
        for (const context of pending) {
            void this.dispatchLaunch(context)
        }
    }

    relaunch (): void {
        window.location.reload()
    }

    quit (): void {
        void this.bridge.invoke('app.quit', {})
    }

    private enqueueLaunch (context: LaunchContext): void {
        if (!this.ready) {
            this.pendingLaunches.push(context)
            return
        }
        void this.dispatchLaunch(context)
    }

    private async dispatchLaunch (context: LaunchContext): Promise<void> {
        if (context.parseError) {
            this.logger.warn('Rejected launch request:', context.parseError)
            return
        }

        if (context.secondInstance || context.request.newWindow) {
            void this.bridge.invoke('window.new', { launch: context }).catch(error => {
                this.logger.warn('Failed to open a launch window:', error)
            })
            return
        }

        const event: CLIEvent = {
            argv: context.request.argv,
            cwd: context.cwd,
            secondInstance: context.secondInstance,
        }
        this.logger.info('CLI arguments received:', event)

        await this.zone.run(async () => {
            const cliHandlers = this.injector.get(CLIHandler) as unknown as CLIHandler[]
            cliHandlers.sort((a, b) => b.priority - a.priority)

            let handled = false
            for (const handler of cliHandlers) {
                if (handled && handler.firstMatchOnly) {
                    continue
                }
                if (await handler.handle(event)) {
                    this.logger.info('CLI handler matched:', handler.constructor.name)
                    handled = true
                }
            }
        })
    }
}
