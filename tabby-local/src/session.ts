import * as fs from 'mz/fs'
import * as fsSync from 'fs'
import { Injector } from '@angular/core'
import { HostAppService, ConfigService, WIN_BUILD_CONPTY_SUPPORTED, isWindowsBuild, Platform, BootstrapData, BOOTSTRAP_DATA, LogService } from 'tabby-core'
import { BaseSession } from 'tabby-terminal'
import { SessionOptions, ChildProcess, PTYInterface, PTYProxy } from './api'
import { getEnvironment, substituteEnv } from './environment'

const windowsDirectoryRegex = /([a-zA-Z]:[^\:\[\]\?\"\<\>\|\x1b]+)/mi

function mergeEnv (...envs) {
    const result = {}
    const keyMap = {}
    for (const env of envs) {
        for (const [key, value] of Object.entries(env)) {
            // const lookup = process.platform === 'win32' ? key.toLowerCase() : key
            const lookup = key.toLowerCase()
            keyMap[lookup] ??= key
            result[keyMap[lookup]] = value
        }
    }
    return result
}

/** @hidden */
export class Session extends BaseSession {
    private pty: PTYProxy|null = null
    private ptyClosed = false
    private pauseAfterExit = false
    private guessedCWD: string|null = null
    private initialCWD: string|null = null
    private config: ConfigService
    private hostApp: HostAppService
    private bootstrapData: BootstrapData
    private ptyInterface: PTYInterface

    constructor (
        injector: Injector,
    ) {
        super(injector.get(LogService).create('local'))
        this.config = injector.get(ConfigService)
        this.hostApp = injector.get(HostAppService)
        this.ptyInterface = injector.get(PTYInterface)
        this.bootstrapData = injector.get(BOOTSTRAP_DATA)
    }

    async start (options: SessionOptions): Promise<void> {
        let pty: PTYProxy|null = null

        if (options.restoreFromPTYID) {
            pty = await this.ptyInterface.restore(options.restoreFromPTYID)
            options.restoreFromPTYID = null
        }

        if (!pty) {
            const baseEnv = getEnvironment(
                this.hostApp.platform === Platform.Windows && this.config.store.terminal.windowsRefreshEnvironment,
            )

            let env = mergeEnv(
                baseEnv,
                {
                    COLORTERM: 'truecolor',
                    TERM: 'xterm-256color',
                    TERM_PROGRAM: 'Tabby',
                },
                substituteEnv(options.env),
                this.config.store.terminal.environment || {},
            )

            if (this.hostApp.platform === Platform.Windows && this.config.store.terminal.setComSpec) {
                env = mergeEnv(env, { COMSPEC: this.bootstrapData.executable })
            }

            delete env['']

            if (this.hostApp.platform === Platform.macOS && !process.env.LC_ALL) {
                const locale = process.env.LC_CTYPE ?? 'en_US.UTF-8'
                Object.assign(env, {
                    LANG: locale,
                    LC_ALL: locale,
                    LC_MESSAGES: locale,
                    LC_NUMERIC: locale,
                    LC_COLLATE: locale,
                    LC_MONETARY: locale,
                })
            }

            // eslint-disable-next-line @typescript-eslint/prefer-nullish-coalescing
            let cwd = options.cwd || process.env.HOME

            if (!fsSync.existsSync(cwd!)) {
                console.warn('Ignoring non-existent CWD:', cwd)
                cwd = undefined
            }

            pty = await this.ptyInterface.spawn(options.command, options.args, {
                name: 'xterm-256color',
                cols: options.width ?? 80,
                rows: options.height ?? 30,
                encoding: null,
                cwd,
                env: env,
                autoSudoPassword: options.autoSudoPassword,
                sudoSecretRef: options.sudoSecretRef,
                // `1` instead of `true` forces ConPTY even if unstable
                useConpty: isWindowsBuild(WIN_BUILD_CONPTY_SUPPORTED) && this.config.store.terminal.useConPTY ? 1 : false,
            })

            this.guessedCWD = cwd ?? null
        }

        this.pty = pty

        pty.getTruePID().then(async () => {
            this.initialCWD = await this.getWorkingDirectory()
        })

        this.open = true

        this.pty.subscribe('data', (array: Uint8Array) => {
            const data = Buffer.from(array)
            const acknowledgedPTY = this.pty
            this.emitOutput(data, () => acknowledgedPTY?.ackData(array.length))
            if (this.hostApp.platform === Platform.Windows) {
                this.guessWindowsCWD(data.toString())
            }
        })

        this.pty.subscribe('exit', () => {
            if (this.pauseAfterExit) {
                return
            } else if (this.open) {
                this.destroy()
            }
        })

        this.pty.subscribe('close', () => {
            this.ptyClosed = true
            if (this.pauseAfterExit) {
                this.emitOutput(Buffer.from('\r\nPress any key to close\r\n'))
            } else if (this.open) {
                this.destroy()
            }
        })

        this.pauseAfterExit = options.pauseAfterExit

        this.destroyed$.subscribe(() => this.pty!.unsubscribeAll())
    }

    getID (): string|null {
        return this.pty?.getID() ?? null
    }

    resize (columns: number, rows: number): void {
        this.pty?.resize(columns, rows)
    }

    write (data: Buffer): void {
        if (this.ptyClosed) {
            this.destroy()
        }
        if (this.open) {
            this.pty?.write(data)
        }
    }

    kill (signal?: string): void {
        void this.pty?.kill(signal).catch(exc => console.info('Could not terminate PTY:', exc))
    }

    async getChildProcesses (): Promise<ChildProcess[]> {
        return this.pty?.getChildProcesses() ?? []
    }

    async gracefullyKillProcess (): Promise<void> {
        const pty = this.pty
        if (!pty) {
            return
        }
        if (this.hostApp.platform === Platform.Windows) {
            try {
                await pty.kill()
            } catch (exc) {
                console.info('Could not terminate PTY:', exc)
            }
            return
        }

        try {
            await pty.kill('SIGTERM')
        } catch (exc) {
            console.info('Could not send SIGTERM to PTY:', exc)
            return
        }

        await new Promise(resolve => setTimeout(resolve, 500))
        try {
            if (await pty.isAlive()) {
                await pty.kill('SIGKILL')
            }
        } catch (exc) {
            console.info('Could not force-terminate PTY:', exc)
        }
    }

    supportsWorkingDirectory (): boolean {
        return !!(this.initialCWD ?? this.reportedCWD ?? this.guessedCWD)
    }

    async getWorkingDirectory (): Promise<string|null> {
        if (this.reportedCWD) {
            return this.reportedCWD
        }
        let cwd: string|null = null
        try {
            cwd = await this.pty?.getWorkingDirectory() ?? null
        } catch (exc) {
            console.info('Could not read working directory:', exc)
        }

        try {
            cwd = await fs.realpath(cwd)
        } catch {}

        if (this.hostApp.platform === Platform.Windows && (cwd === this.initialCWD || cwd === process.env.WINDIR)) {
            // shell doesn't truly change its process' CWD
            cwd = null
        }

        // eslint-disable-next-line @typescript-eslint/prefer-nullish-coalescing
        cwd = cwd || this.guessedCWD

        try {
            await fs.access(cwd)
        } catch {
            return null
        }
        return cwd
    }

    private guessWindowsCWD (data: string) {
        const match = windowsDirectoryRegex.exec(data)
        if (match) {
            this.guessedCWD = match[0]
        }
    }
}
