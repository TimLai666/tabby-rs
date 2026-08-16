import { Injectable } from '@angular/core'
import * as fs from 'fs'
import * as path from 'path'
import { WIN_BUILD_CONPTY_SUPPORTED, isWindowsBuild, HostAppService } from 'tabby-core'
import { SessionOptions, UACService } from 'tabby-local'
import { ElectronService } from './electron.service'

const UAC_PROTOCOL_MARKER = Buffer.from('tabby-rs-uac-', 'utf16le')
const MAX_HELPER_BYTES = 16 * 1024 * 1024

/** @hidden */
@Injectable()
export class ElectronUACService extends UACService {
    private helperPath: string

    constructor (
        private electron: ElectronService,
        private hostApp: HostAppService,
    ) {
        super()
        this.helperPath = this.resolveHelperPath()
        this.isAvailable = isWindowsBuild(WIN_BUILD_CONPTY_SUPPORTED, this.hostApp.platform, this.hostApp.windowsBuild)
            && this.isHardenedHelper(this.helperPath)
    }

    patchSessionOptionsForUAC (sessionOptions: SessionOptions): SessionOptions {
        if (!this.isAvailable) {
            throw new Error('Administrator sessions are unavailable because the hardened UAC helper is missing')
        }

        return {
            ...sessionOptions,
            command: this.helperPath,
            args: [
                '--cwd',
                sessionOptions.cwd ?? '',
                '--',
                sessionOptions.command,
                ...sessionOptions.args,
            ],
        }
    }

    private isHardenedHelper (helperPath: string): boolean {
        try {
            const metadata = fs.lstatSync(helperPath)
            if (!metadata.isFile() || metadata.isSymbolicLink() || metadata.size > MAX_HELPER_BYTES) {
                return false
            }
            return fs.readFileSync(helperPath).includes(UAC_PROTOCOL_MARKER)
        } catch {
            return false
        }
    }

    private resolveHelperPath (): string {
        if (process.env.TABBY_DEV) {
            return path.join(
                path.dirname(this.electron.app.getPath('exe')),
                '..', '..', '..',
                'extras',
                'UAC.exe',
            )
        }

        return path.join(
            path.dirname(this.electron.app.getPath('exe')),
            'resources',
            'extras',
            'UAC.exe',
        )
    }
}
