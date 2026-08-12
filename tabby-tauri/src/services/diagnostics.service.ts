import { Injectable } from '@angular/core'

import {
    DiagnosticsExportRequest,
    DiagnosticsOptions,
    DiagnosticsPreview,
    DiagnosticsStatus,
    HostBridge,
} from '../api/hostBridge'

@Injectable()
export class TauriDiagnosticsService {
    constructor (private bridge: HostBridge) { }

    status (): Promise<DiagnosticsStatus> {
        return this.bridge.invoke('diagnostics.status', {})
    }

    preview (options: DiagnosticsOptions = {}): Promise<DiagnosticsPreview> {
        return this.bridge.invoke('diagnostics.preview', options)
    }

    async exportBundle (options: DiagnosticsOptions = {}): Promise<string|null> {
        const destination = await this.bridge.invoke('dialog.save', {
            title: 'Export diagnostics',
            fileName: 'tabby-rs-diagnostics.zip',
        })
        if (!destination) {
            return null
        }
        const request: DiagnosticsExportRequest = { destination, ...options }
        return this.bridge.invoke('diagnostics.export', request)
    }

    clearLogs (): Promise<null> {
        return this.bridge.invoke('diagnostics.clearLogs', {})
    }
}
