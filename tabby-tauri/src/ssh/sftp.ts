import { FileDownload, FileUpload } from 'tabby-core'

import { HostBridge, RemoteFileEntry, SftpTransferDescriptor } from '../api/hostBridge'

export class TauriSftpSession {
    private closed = false

    private constructor (
        private bridge: HostBridge,
        public readonly id: string,
    ) { }

    static async open (bridge: HostBridge, sshSessionId: string): Promise<TauriSftpSession> {
        const info = await bridge.invoke('sftp.open', { id: sshSessionId })
        return new TauriSftpSession(bridge, info.id)
    }

    async readdir (path: string): Promise<RemoteFileEntry[]> {
        this.ensureOpen()
        return this.bridge.invoke('sftp.list', { id: this.id, path })
    }

    async stat (path: string, follow = false): Promise<RemoteFileEntry> {
        this.ensureOpen()
        return this.bridge.invoke('sftp.stat', { id: this.id, path, follow })
    }

    async mkdir (path: string): Promise<void> {
        this.ensureOpen()
        await this.bridge.invoke('sftp.mkdir', { id: this.id, path })
    }

    async rename (from: string, to: string): Promise<void> {
        this.ensureOpen()
        await this.bridge.invoke('sftp.rename', { id: this.id, from, to })
    }

    async remove (path: string, recursive = false): Promise<void> {
        this.ensureOpen()
        await this.bridge.invoke('sftp.remove', { id: this.id, path, recursive })
    }

    async upload (path: string, transfer: FileUpload, overwritePolicy: 'skip'|'overwrite'|'rename' = 'skip'): Promise<SftpTransferDescriptor> {
        this.ensureOpen()
        const descriptor = await this.bridge.invoke('sftp.uploadOpen', {
            id: this.id,
            path,
            size: transfer.getSize(),
            overwritePolicy,
        })
        try {
            while (true) {
                const chunk = await transfer.read()
                if (!chunk.length) {
                    break
                }
                await this.bridge.invoke('sftp.write', {
                    id: this.id,
                    transferId: descriptor.id,
                    data: Array.from(chunk),
                })
            }
            const result = await this.bridge.invoke('sftp.closeTransfer', { id: this.id, transferId: descriptor.id })
            await transfer.closeAsync()
            return result
        } catch (error) {
            transfer.cancel()
            await this.bridge.invoke('sftp.cancelTransfer', { id: this.id, transferId: descriptor.id }).catch(() => undefined)
            throw error
        }
    }

    async download (path: string, transfer: FileDownload): Promise<SftpTransferDescriptor> {
        this.ensureOpen()
        const descriptor = await this.bridge.invoke('sftp.downloadOpen', { id: this.id, path })
        try {
            while (true) {
                const chunk = await this.bridge.invoke('sftp.read', {
                    id: this.id,
                    transferId: descriptor.id,
                    maxBytes: 256 * 1024,
                })
                if (!chunk.length) {
                    break
                }
                await transfer.write(Uint8Array.from(chunk))
            }
            const result = await this.bridge.invoke('sftp.closeTransfer', { id: this.id, transferId: descriptor.id })
            await transfer.closeAsync()
            return result
        } catch (error) {
            transfer.cancel()
            await this.bridge.invoke('sftp.cancelTransfer', { id: this.id, transferId: descriptor.id }).catch(() => undefined)
            throw error
        }
    }

    async close (): Promise<void> {
        if (this.closed) {
            return
        }
        this.closed = true
        await this.bridge.invoke('sftp.close', { id: this.id })
    }

    private ensureOpen (): void {
        if (this.closed) {
            throw new Error('SFTP session is closed')
        }
    }
}
