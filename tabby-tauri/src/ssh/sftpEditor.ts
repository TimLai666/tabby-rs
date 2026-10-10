import { Subscription } from 'rxjs'
import { Injectable } from '@angular/core'
import { FileDownload, FileUpload, NotificationsService } from 'tabby-core'
import { SFTPFile } from '../../../tabby-ssh/src/api/sftp'
import { HostBridge } from '../api/hostBridge'
import { TauriPlatformService } from '../services/platform.service'
import { TauriSftpPanelTransport } from './sftpPanelTransport'

@Injectable({ providedIn: 'root' })
export class TauriSftpEditor {
    constructor (private bridge: HostBridge, private platform: TauriPlatformService, private notifications: NotificationsService) { }

    async edit (item: SFTPFile, sftp: TauriSftpPanelTransport): Promise<void> {
        let workspace: { id: string; path: string; transfer: FileDownload }|undefined = undefined
        const state = { stopped: false, stopSent: false, downloading: false, detached: false }
        let unlisten: (() => void)|undefined = undefined
        let armTimer: ReturnType<typeof setTimeout>|undefined = undefined
        let saveTimer: ReturnType<typeof setTimeout>|undefined = undefined
        let activeUpload: FileUpload|undefined = undefined
        let uploads = Promise.resolve()
        const isStopped = () => state.stopped || sftp.isClosed()
        const report = (error: unknown) => {
            const value = error as { details?: string; message?: string }|null
            this.notifications.error(value?.details ?? value?.message ?? String(error))
        }
        const detach = () => {
            state.detached = true
            unlisten?.()
            unlisten = undefined
            if (workspace && !state.stopSent) {
                state.stopSent = true
                void this.bridge.invoke('fileEdit.stop', { id: workspace.id }).catch(report)
            }
        }
        let closeSubscription: Subscription|undefined = undefined
        const stop = () => {
            state.stopped = true
            clearTimeout(armTimer)
            clearTimeout(saveTimer)
            if (state.downloading) { workspace?.transfer.cancel() }
            activeUpload?.cancel()
            detach()
            closeSubscription?.unsubscribe()
        }
        closeSubscription = sftp.closed$.subscribe(() => stop())
        const queueSave = (rename: boolean) => {
            if (isStopped()) { return }
            if (rename) { detach() }
            uploads = uploads.then(async () => {
                if (isStopped() || !workspace) { return }
                const selected = await this.platform.startUpload({ multiple: false }, [workspace.path])
                if (isStopped()) { selected.forEach(file => file.cancel()); return }
                if (!selected.length) { return }
                activeUpload = selected[0]
                try {
                    await sftp.upload(item.fullPath, activeUpload, 'overwrite')
                    if (!isStopped() && !sftp.isClosed()) { await sftp.chmod(item.fullPath, item.mode) }
                } finally { activeUpload = undefined }
            }).catch(error => { if (!isStopped()) { report(error) } }).finally(() => { if (rename) { stop() } })
        }
        try {
            if (sftp.isClosed()) { stop(); return }
            workspace = await this.platform.prepareEditableFile(item.name, item.mode, item.size)
            state.downloading = true
            if (isStopped()) { stop(); return }
            await sftp.download(item.fullPath, workspace.transfer)
            state.downloading = false
            if (isStopped()) { stop(); return }
            await this.bridge.invoke('fileEdit.ready', { id: workspace.id })
            if (isStopped()) { stop(); return }
            await this.bridge.invoke('desktop.openPath', { path: workspace.path })
            if (isStopped()) { stop(); return }
            // Match the original editor's initial-event grace period and save debounce.
            armTimer = setTimeout(() => {
                void (async () => {
                    if (isStopped() || !workspace) { return }
                    unlisten = await this.bridge.listen('fileEdit:changed', event => {
                        if (isStopped() || state.detached || event.id !== workspace?.id) { return }
                        if (event.event === 'error') { report(event.message ?? 'File watcher failed'); stop(); return }
                        clearTimeout(saveTimer)
                        saveTimer = setTimeout(() => queueSave(event.event === 'rename'), 1000)
                    })
                    if (isStopped()) { detach(); return }
                    await this.bridge.invoke('fileEdit.watch', { id: workspace.id })
                    if (isStopped()) { detach() }
                })().catch(error => {
                    if (!isStopped()) { report(error) }
                    stop()
                })
            }, 1000)
        } catch (error) {
            if (!isStopped()) { report(error) }
            stop()
        }
    }
}
