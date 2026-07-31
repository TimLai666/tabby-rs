import { Inject, Injectable } from '@angular/core'
import {
    BOOTSTRAP_DATA,
    BootstrapData,
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
    private configContent: string

    constructor (
        private bridge: HostBridge,
        @Inject(BOOTSTRAP_DATA) bootstrapData: BootstrapData,
        @Inject(TAURI_RUNTIME_INFO) private runtimeInfo: RuntimeInfo,
    ) {
        super()
        this.configContent = JSON.stringify(bootstrapData.config ?? {}, null, 2)
    }

    readClipboard (): string {
        return this.clipboardText
    }

    setClipboard (content: ClipboardContent): void {
        this.clipboardText = content.text
        if (navigator.clipboard?.writeText) {
            void navigator.clipboard.writeText(content.text).catch(() => null)
        }
    }

    async loadConfig (): Promise<string> {
        return this.configContent
    }

    async saveConfig (content: string): Promise<void> {
        this.configContent = content
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
        const parsed = new URL(url)
        if (!['http:', 'https:', 'mailto:'].includes(parsed.protocol)) {
            throw new Error(`Unsupported URL scheme: ${parsed.protocol}`)
        }
        window.open(parsed.toString(), '_blank', 'noopener,noreferrer')
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
        console.warn('Native context menus are not implemented by the Tauri foundation yet')
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
                ? (options.defaultId ?? 0)
                : (options.cancelId ?? options.buttons.length - 1),
        }
    }

    async pickDirectory (): Promise<string|null> {
        return null
    }

    quit (): void {
        void this.bridge.invoke('app.quit', {})
    }

    getTheme (): PlatformTheme {
        return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light'
    }
}
