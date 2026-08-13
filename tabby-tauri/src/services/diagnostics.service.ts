import { Injectable } from '@angular/core'
import { TranslateService } from '@ngx-translate/core'

import { PlatformService } from 'tabby-core'

import {
    DiagnosticsExportRequest,
    DiagnosticsOptions,
    DiagnosticsPreview,
    DiagnosticsStatus,
    HostBridge,
} from '../api/hostBridge'

@Injectable()
export class TauriDiagnosticsService {
    constructor (
        private bridge: HostBridge,
        private platform: PlatformService,
        private translate: TranslateService,
    ) { }

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
        const confirmation = await this.platform.showMessageBox({
            type: 'warning',
            message: this.translate.instant('Export the reviewed diagnostic files?'),
            detail: this.translate.instant('The ZIP will contain the files shown in the preview after redaction.'),
            buttons: [
                this.translate.instant('Export'),
                this.translate.instant('Cancel'),
            ],
            defaultId: 0,
            cancelId: 1,
        })
        if (confirmation.response !== 0) {
            return null
        }
        const request: DiagnosticsExportRequest = { destination, ...options }
        return this.bridge.invoke('diagnostics.export', request)
    }

    clearLogs (): Promise<null> {
        return this.bridge.invoke('diagnostics.clearLogs', {})
    }
}
