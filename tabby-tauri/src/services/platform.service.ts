import { Inject, Injectable, NgZone } from '@angular/core'
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
    private contextMenuElement: HTMLElement | null = null
    private contextMenuCleanup: (() => void) | null = null

    constructor (
        private bridge: HostBridge,
        @Inject(TAURI_RUNTIME_INFO) private runtimeInfo: RuntimeInfo,
        private zone: NgZone,
    ) {
        super()
        void this.initializeDesktopEvents()
    }

    readClipboard (): string {
        void this.refreshClipboard()
        return this.clipboardText
    }

    async readClipboardText (): Promise<string> {
        try {
            this.clipboardText = await this.bridge.invoke('clipboard.readText', {})
        } catch (error) {
            console.warn('Could not read native clipboard', error)
        }
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

    popupContextMenu (menu: MenuItemOptions[], event?: MouseEvent): void {
        this.closeContextMenu()

        const root = document.createElement('div')
        root.setAttribute('role', 'menu')
        Object.assign(root.style, {
            background: 'var(--bs-body-bg, #202124)',
            border: '1px solid var(--bs-border-color, #555)',
            borderRadius: '4px',
            boxShadow: '0 4px 16px rgba(0, 0, 0, .35)',
            color: 'var(--bs-body-color, #eee)',
            fontFamily: 'inherit',
            fontSize: '13px',
            minWidth: '180px',
            padding: '4px 0',
            position: 'fixed',
            zIndex: '2147483647',
        })

        this.renderMenuItems(root, menu)
        document.body.appendChild(root)

        const x = event?.clientX ?? 0
        const y = event?.clientY ?? 0
        root.style.left = `${Math.max(0, Math.min(x, window.innerWidth - root.offsetWidth))}px`
        root.style.top = `${Math.max(0, Math.min(y, window.innerHeight - root.offsetHeight))}px`

        const closeOnOutsideClick = (clickEvent: MouseEvent) => {
            if (!root.contains(clickEvent.target as Node)) {
                this.closeContextMenu()
            }
        }
        const closeOnEscape = (keyEvent: KeyboardEvent) => {
            if (keyEvent.key === 'Escape') {
                keyEvent.preventDefault()
                this.closeContextMenu()
            }
        }
        document.addEventListener('mousedown', closeOnOutsideClick)
        document.addEventListener('keydown', closeOnEscape)
        this.contextMenuElement = root
        this.contextMenuCleanup = () => {
            document.removeEventListener('mousedown', closeOnOutsideClick)
            document.removeEventListener('keydown', closeOnEscape)
        }
    }

    private renderMenuItems (container: HTMLElement, items: MenuItemOptions[]): void {
        for (const item of items) {
            if (item.type === 'separator') {
                const separator = document.createElement('div')
                separator.setAttribute('role', 'separator')
                Object.assign(separator.style, {
                    borderTop: '1px solid var(--bs-border-color, #555)',
                    margin: '4px 0',
                })
                container.appendChild(separator)
                continue
            }

            const row = document.createElement('div')
            row.setAttribute('role', 'none')
            Object.assign(row.style, {
                position: 'relative',
            })

            const button = document.createElement('button')
            button.type = 'button'
            button.setAttribute('role', 'menuitem')
            button.disabled = item.enabled === false
            button.textContent = `${item.checked ? '✓ ' : item.type === 'radio' ? '○ ' : ''}${item.label ?? ''}`
            if (item.sublabel ?? item.commandLabel) {
                const suffix = document.createElement('span')
                suffix.textContent = item.sublabel ?? item.commandLabel ?? ''
                suffix.style.marginLeft = 'auto'
                suffix.style.opacity = '0.65'
                button.appendChild(suffix)
            }
            if (item.submenu) {
                button.setAttribute('aria-haspopup', 'menu')
                const arrow = document.createElement('span')
                arrow.textContent = '›'
                arrow.style.marginLeft = 'auto'
                button.appendChild(arrow)
            }
            Object.assign(button.style, {
                background: 'transparent',
                border: '0',
                color: 'inherit',
                cursor: item.enabled === false ? 'default' : 'pointer',
                display: 'flex',
                padding: '6px 12px',
                textAlign: 'left',
                width: '100%',
            })
            button.addEventListener('mouseenter', () => {
                if (!button.disabled) {
                    button.style.background = 'var(--bs-primary, #375a9e)'
                }
            })
            button.addEventListener('mouseleave', () => {
                button.style.background = 'transparent'
            })
            if (!item.submenu) {
                button.addEventListener('click', () => {
                    this.closeContextMenu()
                    this.zone.run(() => item.click?.())
                })
            }
            row.appendChild(button)

            if (item.submenu) {
                const submenu = document.createElement('div')
                submenu.setAttribute('role', 'menu')
                Object.assign(submenu.style, {
                    background: 'var(--bs-body-bg, #202124)',
                    border: '1px solid var(--bs-border-color, #555)',
                    borderRadius: '4px',
                    boxShadow: '0 4px 16px rgba(0, 0, 0, .35)',
                    display: 'none',
                    left: '100%',
                    minWidth: '180px',
                    padding: '4px 0',
                    position: 'absolute',
                    top: '0',
                    zIndex: '1',
                })
                this.renderMenuItems(submenu, item.submenu)
                row.addEventListener('mouseenter', () => { submenu.style.display = 'block' })
                row.addEventListener('mouseleave', () => { submenu.style.display = 'none' })
                row.appendChild(submenu)
            }

            container.appendChild(row)
        }
    }

    private closeContextMenu (): void {
        this.contextMenuCleanup?.()
        this.contextMenuCleanup = null
        this.contextMenuElement?.remove()
        this.contextMenuElement = null
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
