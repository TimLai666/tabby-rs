import { Injectable } from '@angular/core'
import * as fs from 'fs'
import * as path from 'path'
import { WIN_BUILD_CONPTY_SUPPORTED, isWindowsBuild } from 'tabby-core'
import { SessionOptions, UACService } from 'tabby-local'
import { ElectronService } from './electron.service'

/** @hidden */
@Injectable()
export class ElectronUACService extends UACService {
    private helperPath: string

    constructor (
        private electron: ElectronService,
    ) {
        super()
        this.helperPath = this.resolveHelperPath()
        this.isAvailable = isWindowsBuild(WIN_BUILD_CONPTY_SUPPORTED)
            && fs.existsSync(this.helperPath)
    }

    patchSessionOptionsForUAC (sessionOptions: SessionOptions): SessionOptions {
        if (!this.isAvailable) {
            throw new Error('Administrator sessions are unavailable because the UAC helper is missing')
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
