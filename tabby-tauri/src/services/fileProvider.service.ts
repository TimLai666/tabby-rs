import { Injectable } from '@angular/core'
import { FileProvider } from 'tabby-core'

import { HostBridge } from '../api/hostBridge'

const FILE_URI_PREFIX = 'file://'

@Injectable()
export class TauriFileProvider extends FileProvider {
    name = 'Filesystem'

    constructor (private bridge: HostBridge) {
        super()
    }

    async selectAndStoreFile (description: string): Promise<string> {
        const [path] = await this.bridge.invoke('dialog.open', {
            multiple: false,
            directory: false,
            title: description,
        })
        if (!path) {
            throw new Error('File selection cancelled')
        }
        return `${FILE_URI_PREFIX}${path}`
    }

    async retrieveFile (key: string): Promise<Buffer> {
        if (!key.startsWith(FILE_URI_PREFIX)) {
            throw new Error('Unsupported file reference')
        }
        const bytes = await this.bridge.invoke('desktop.readFile', {
            path: key.slice(FILE_URI_PREFIX.length),
        })
        return Buffer.from(bytes)
    }
}
