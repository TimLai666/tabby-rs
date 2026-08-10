import { Inject, Injectable } from '@angular/core'
import {
    ClipboardContent,
    DirectoryDownload,
    DirectoryUpload,
    FileDownload,
    FileUpload,
    FileUploadOptions,
    MenuItemOptions,
    MessageBoxOptions,
    MessageBoxResult,
    PlatformService,
    PlatformTheme,
} from 'tabby-core'

import { HostBridge, RuntimeInfo, TAURI_RUNTIME_INFO } from '../api/hostBridge'

@Injectable()
export class TauriPlatformService extends PlatformService {
    private clipboardText = ''
    private configRevision: string | null = null
    private configPath: string | null = null
    private theme: PlatformTheme = window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light'

    constructor (
        private bridge: HostBridge,
        @Inject(TAURI_RUNTIME_INFO) private runtimeInfo: RuntimeInfo,
    ) {
        super()
        void this.initializeDesktopEvents()
    }

    readClipboard (): string {
        void this.refreshClipboard()
        return this.clipboardText
    }

    setClipboard (content: ClipboardContent): void {
        this.clipboardText = content.text
        void this.bridge.invoke('clipboard.writeText', { text: content.text })
            .catch(error => console.warn('Could not write native clipboard', error))
    }

    async loadConfig (): Promise<string> {
        const result = await this.bridge.invoke('config.read', {})
        this.configRevision = result.revision
        this.configPath = result.path
        return result.yaml
    }

    async saveConfig (content: string): Promise<void> {
        const result = await this.bridge.invoke('config.write', {
            yaml: content,
            expectedRevision: this.configRevision,
            requireMissing: this.configRevision === null,
        })
        this.configRevision = result.revision
        this.configPath = result.path
    }

    getConfigPath (): string | null {
        return this.configPath
    }

    async startDownload (_name: string, _mode: number, _size: number): Promise<FileDownload|null> {
        return null
    }

    async startDownloadDirectory (_name: string, _estimatedSize?: number): Promise<DirectoryDownload|null> {
        return null
    }

    async startUpload (_options?: FileUploadOptions): Promise<FileUpload[]> {
        return []
    }

    async startUploadDirectory (_paths?: string[]): Promise<DirectoryUpload> {
        return new DirectoryUpload()
    }

    getOSRelease (): string {
        return `${this.runtimeInfo.platform}/${this.runtimeInfo.arch}`
    }

    getAppVersion (): string {
        return this.runtimeInfo.version
    }

    async openExternal (url: string): Promise<void> {
        await this.bridge.invoke('desktop.openExternal', { url })
    }

    showItemInFolder (path: string): void {
        void this.bridge.invoke('desktop.revealPath', { path })
            .catch(error => console.warn('Could not reveal path', error))
    }

    openPath (path: string): void {
        void this.bridge.invoke('desktop.openPath', { path })
            .catch(error => console.warn('Could not open path', error))
    }

    async listFonts (): Promise<string[]> {
        return []
    }

    setErrorHandler (handler: (_: any) => void): void {
        window.onerror = (_message, _source, _line, _column, error) => {
            handler(error ?? _message)
            return false
        }
        window.onunhandledrejection = event => {
            handler(event.reason)
        }
    }

    popupContextMenu (_menu: MenuItemOptions[], _event?: MouseEvent): void {
        console.warn('Native context menus are not implemented by the Tauri desktop integration yet')
    }

    async showMessageBox (options: MessageBoxOptions): Promise<MessageBoxResult> {
        const text = [options.message, options.detail].filter(Boolean).join('\n\n')
        if (options.buttons.length <= 1) {
            window.alert(text)
            return { response: options.defaultId ?? 0 }
        }
        const accepted = window.confirm(text)
        return {
            response: accepted
                ? options.defaultId ?? 0
                : options.cancelId ?? options.buttons.length - 1,
        }
    }

    async pickDirectory (): Promise<string|null> {
        const paths = await this.bridge.invoke('dialog.open', {
            multiple: false,
            directory: true,
            title: null,
        })
        return paths.length ? paths[0] : null
    }

    quit (): void {
        void this.bridge.invoke('app.quit', {})
    }

    getTheme (): PlatformTheme {
        return this.theme
    }

    private async initializeDesktopEvents (): Promise<void> {
        await Promise.all([
            this.bridge.listen('desktop.displayMetricsChanged', () => this.displayMetricsChanged.next()),
            this.bridge.listen('desktop.themeChanged', theme => {
                const next = theme === 'system'
                    ? window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light'
                    : theme
                if (next !== this.theme) {
                    this.theme = next
                    this.themeChanged.next(next)
                }
            }),
        ])
        await this.refreshClipboard()
    }

    private async refreshClipboard (): Promise<void> {
        try {
            this.clipboardText = await this.bridge.invoke('clipboard.readText', {})
        } catch {
            // Clipboard reads can fail while another application owns the clipboard.
        }
    }
}
