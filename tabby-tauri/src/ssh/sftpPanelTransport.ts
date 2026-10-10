import { FileDownload, FileUpload } from 'tabby-core'
import { SFTPFile } from '../../../tabby-ssh/src/api/sftp'
import { SFTPPanelTransport } from '../../../tabby-ssh/src/components/sftpPanel.controller'
import { RemoteFileEntry } from '../api/hostBridge'
import { TauriSftpSession } from './sftp'

export interface TauriSftpPanelFile extends SFTPFile {
    isOperable: boolean
    unoperableReason?: string|null
}

function toPanelFile (entry: RemoteFileEntry): TauriSftpPanelFile {
    return { ...entry, modified: new Date((entry.modified ?? 0) * 1000) }
}

function toPanelError (error: unknown): Error {
    if (error instanceof Error) { return error }
    const details = typeof error === 'object' && error !== null && 'details' in error ? error.details : null
    const message = typeof error === 'object' && error !== null && 'message' in error ? error.message : null
    return new Error(typeof details === 'string' ? details : typeof message === 'string' ? message : String(error))
}

export class TauriSftpPanelTransport implements SFTPPanelTransport {
    constructor (private session: TauriSftpSession) { }

    async readdir (path: string): Promise<TauriSftpPanelFile[]> {
        try { return (await this.session.readdir(path)).map(toPanelFile) } catch (error) { throw toPanelError(error) }
    }

    async stat (path: string, follow = true): Promise<TauriSftpPanelFile> {
        try { return toPanelFile(await this.session.stat(path, follow)) } catch (error) { throw toPanelError(error) }
    }

    readlink (path: string): Promise<string> {
        return this.session.readlink(path)
    }

    mkdir (path: string): Promise<void> {
        return this.session.mkdir(path)
    }

    upload (path: string, transfer: FileUpload, policy: 'skip'|'overwrite'|'rename' = 'skip'): Promise<unknown> {
        return this.session.upload(path, transfer, policy)
    }

    remove (path: string, recursive = false): Promise<void> {
        return this.session.remove(path, recursive)
    }

    download (path: string, transfer: FileDownload): Promise<unknown> {
        return this.session.download(path, transfer)
    }
}
