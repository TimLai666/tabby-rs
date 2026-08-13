import { Component, Injectable } from '@angular/core'
import { SettingsTabProvider } from 'tabby-settings'

import { DiagnosticsPreview, DiagnosticsStatus } from '../api/hostBridge'
import { TauriDiagnosticsService } from '../services/diagnostics.service'

@Component({
    selector: 'tauri-diagnostics-settings-tab',
    templateUrl: './diagnosticsSettingsTab.component.pug',
    styles: [`
        :host { display: block; }
        pre { max-height: 240px; overflow: auto; white-space: pre-wrap; }
    `],
})
export class TauriDiagnosticsSettingsTabComponent {
    status: DiagnosticsStatus|null = null
    preview: DiagnosticsPreview|null = null
    busy = false
    message: string|null = null

    constructor (private diagnostics: TauriDiagnosticsService) { }

    async ngOnInit (): Promise<void> {
        await this.refresh()
    }

    async refresh (): Promise<void> {
        try {
            this.status = await this.diagnostics.status()
        } catch (error) {
            this.message = error instanceof Error ? error.message : String(error)
        }
    }

    async loadPreview (): Promise<void> {
        this.busy = true
        this.message = null
        try {
            this.preview = await this.diagnostics.preview()
        } catch (error) {
            this.message = error instanceof Error ? error.message : String(error)
        } finally {
            this.busy = false
        }
    }

    async exportBundle (): Promise<void> {
        this.busy = true
        this.message = null
        try {
            if (!this.preview) {
                this.message = 'Preview diagnostics before exporting.'
                return
            }
            const exported = await this.diagnostics.exportBundle()
            if (exported) {
                this.message = `Diagnostics exported to ${exported}`
            }
        } catch (error) {
            this.message = error instanceof Error ? error.message : String(error)
        } finally {
            this.busy = false
        }
    }

    async clearLogs (): Promise<void> {
        this.busy = true
        this.message = null
        try {
            await this.diagnostics.clearLogs()
            await this.refresh()
            this.preview = null
            this.message = 'Local diagnostic logs cleared.'
        } catch (error) {
            this.message = error instanceof Error ? error.message : String(error)
        } finally {
            this.busy = false
        }
    }
}

@Injectable()
export class TauriDiagnosticsSettingsTabProvider extends SettingsTabProvider {
    id = 'diagnostics'
    icon = 'user-shield'
    title = 'Diagnostics'

    getComponentType (): any {
        return TauriDiagnosticsSettingsTabComponent
    }
}
