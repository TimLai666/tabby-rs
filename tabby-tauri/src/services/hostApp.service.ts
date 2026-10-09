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
    private readonly detectedWindowsBuild: number|undefined

    get windowsBuild (): number|undefined {
        return this.detectedWindowsBuild
    }

    private ready = false
    private pendingLaunches: LaunchContext[] = []
    private launchRead: Promise<void> = Promise.resolve()

    constructor (
        private injector: Injector,
        private zone: NgZone,
        private bridge: HostBridge,
        @Inject(TAURI_RUNTIME_INFO) runtimeInfo: RuntimeInfo,
    ) {
        super(injector)
        this.platform = mapPlatform(runtimeInfo.platform)
        this.configPlatform = this.platform
        this.detectedWindowsBuild = runtimeInfo.windowsBuild ?? undefined

        void this.bridge.listen('app:launch', () => this.readLaunchRequests()).then(() => {
            this.readLaunchRequests()
        }).catch(error => {
            this.logger.error('Failed to listen for launch requests:', error)
            this.readLaunchRequests()
        })
    }

    newWindow (): void {
        void this.bridge.invoke('window.new', {}).catch(error => {
            this.logger.warn('Failed to open a new window:', error)
        })
    }

    emitReady (): void {
        void this.bridge.invoke('window.applyState', { visible: true }).catch(error => {
            this.logger.warn('Failed to show ready window:', error)
        })
        this.ready = true
        const pending = this.pendingLaunches.splice(0)
        for (const context of pending) {
            this.enqueueLaunch(context)
        }
    }

    relaunch (): void {
        window.location.reload()
    }

    quit (): void {
        void this.bridge.invoke('app.quit', {})
    }

    private readLaunchRequests (): void {
        this.launchRead = this.launchRead.then(async () => {
            while (true) {
                const context = await this.bridge.invoke('app.initialLaunch', {})
                if (!context) {
                    return
                }
                this.enqueueLaunch(context)
            }
        }).catch(error => {
            this.logger.error('Failed to read launch requests:', error)
        })
    }

    private enqueueLaunch (context: LaunchContext): void {
        if (!this.ready) {
            this.pendingLaunches.push(context)
            return
        }
        void this.dispatchLaunch(context).catch(error => {
            this.logger.error('Failed to handle launch request:', error)
        })
    }

    private async dispatchLaunch (context: LaunchContext): Promise<void> {
        if (context.parseError) {
            this.logger.warn('Rejected launch request:', context.parseError)
            return
        }

        if (context.request.newWindow) {
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
