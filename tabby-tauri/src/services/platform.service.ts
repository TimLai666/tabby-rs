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
    NodeToolchainStatus,
    PluginInfo,
    PlatformService,
    PlatformTheme,
    sanitizeTransferName,
    sanitizeTransferRelativePath,
} from 'tabby-core'

import {
    HostBridge,
    PluginOperation,
    RuntimeInfo,
    TAURI_RUNTIME_INFO,
    TransferDirectoryEntry,
} from '../api/hostBridge'

@Injectable()
export class TauriPlatformService extends PlatformService {
    supportsPluginManagement = false
    private clipboardText = ''
    private configRevision: string | null = null
    private configPath: string | null = null
    private customNodePath: string | null = null
    private activePluginOperations = new Map<string, string>()
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

    async startDownload (name: string, mode: number, size: number): Promise<FileDownload|null> {
        const path = await this.bridge.invoke('dialog.save', {
            fileName: sanitizeTransferName(name),
            title: null,
        })
        if (!path) {
            return null
        }
        const descriptor = await this.bridge.invoke('transfer.openDownload', {
            name,
            mode,
            size,
            destination: path,
        })
        const transfer = new TauriFileDownload(this.bridge, descriptor.id, descriptor.name, size)
        this.fileTransferStarted.next(transfer)
        return transfer
    }

    async startDownloadDirectory (name: string, estimatedSize = 0): Promise<DirectoryDownload|null> {
        const basePath = await this.pickDirectory()
        if (!basePath) {
            return null
        }
        const transfer = new TauriDirectoryDownload(this.bridge, basePath, name, estimatedSize)
        this.fileTransferStarted.next(transfer)
        return transfer
    }

    async startUpload (options: FileUploadOptions = { multiple: false }): Promise<FileUpload[]> {
        const paths = await this.bridge.invoke('dialog.open', {
            multiple: options.multiple,
            directory: false,
            title: null,
        })
        if (!paths.length) {
            return []
        }
        const descriptors = await this.bridge.invoke('transfer.openUpload', { paths })
        const transfers = descriptors.map(descriptor => new TauriFileUpload(this.bridge, descriptor.id, descriptor.name, descriptor.size ?? 0))
        transfers.forEach(transfer => this.fileTransferStarted.next(transfer))
        return transfers
    }

    async startUploadDirectory (paths?: string[]): Promise<DirectoryUpload> {
        if (!paths?.length) {
            paths = await this.bridge.invoke('dialog.open', {
                multiple: false,
                directory: true,
                title: null,
            })
        }
        if (!paths.length) {
            return new DirectoryUpload()
        }

        const tree = await this.bridge.invoke('transfer.listDirectory', { path: paths[0] })
        const files: TransferDirectoryEntry[] = []
        const collect = (entry: TransferDirectoryEntry) => {
            if (entry.directory) {
                entry.children.forEach(collect)
            } else {
                files.push(entry)
            }
        }
        collect(tree)
        const descriptors = await this.bridge.invoke('transfer.openUpload', { paths: files.map(file => file.path) })
        let index = 0
        const build = (entry: TransferDirectoryEntry): FileUpload|DirectoryUpload => {
            if (!entry.directory) {
                const descriptor = descriptors[index++]
                const transfer = new TauriFileUpload(this.bridge, descriptor.id, descriptor.name, descriptor.size ?? entry.size)
                this.fileTransferStarted.next(transfer)
                return transfer
            }
            const directory = new DirectoryUpload(entry.name)
            entry.children.forEach(child => directory.pushChildren(build(child)))
            return directory
        }
        return build(tree) as DirectoryUpload
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

    async exec (app: string, argv: string[]): Promise<void> {
        await this.bridge.invoke('desktop.exec', { executable: app, args: argv })
    }

    getWinSCPPath (): string | null {
        return null
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
        try {
            const fonts = await this.bridge.invoke('font.list', {})
            return fonts.map(font => font.family)
        } catch (error) {
            console.warn('Could not enumerate installed fonts', error)
            return []
        }
    }

    async getNodeToolchainStatus (customNodePath?: string): Promise<NodeToolchainStatus> {
        const trimmedNodePath = customNodePath?.trim()
        this.customNodePath = trimmedNodePath ? trimmedNodePath : null
        try {
            const status = await this.bridge.invoke('plugins.nodeStatus', { customNodePath: this.customNodePath })
            this.supportsPluginManagement = status.supported
            return status
        } catch (error) {
            this.supportsPluginManagement = false
            throw error
        }
    }

    async installPlugin (name: string, version: string): Promise<void> {
        const operationId = await this.beginPluginOperation(name)
        let watcher: { result: Promise<PluginOperation>; dispose: () => void }|null = null
        try {
            watcher = await this.watchPluginOperation(operationId)
            await this.bridge.invoke('plugins.install', {
                operationId,
                packageName: name,
                version,
                customNodePath: this.customNodePath,
            })
            this.requireSuccessfulPluginOperation(await watcher.result)
        } finally {
            watcher?.dispose()
            if (this.activePluginOperations.get(name) === operationId) {
                this.activePluginOperations.delete(name)
            }
        }
    }

    async updatePlugin (name: string): Promise<void> {
        const operationId = await this.beginPluginOperation(name)
        let watcher: { result: Promise<PluginOperation>; dispose: () => void }|null = null
        try {
            watcher = await this.watchPluginOperation(operationId)
            await this.bridge.invoke('plugins.update', {
                operationId,
                packageName: name,
                customNodePath: this.customNodePath,
            })
            this.requireSuccessfulPluginOperation(await watcher.result)
        } finally {
            watcher?.dispose()
            if (this.activePluginOperations.get(name) === operationId) {
                this.activePluginOperations.delete(name)
            }
        }
    }

    async uninstallPlugin (name: string): Promise<void> {
        const operationId = await this.beginPluginOperation(name)
        let watcher: { result: Promise<PluginOperation>; dispose: () => void }|null = null
        try {
            watcher = await this.watchPluginOperation(operationId)
            await this.bridge.invoke('plugins.uninstall', {
                operationId,
                packageName: name,
                customNodePath: this.customNodePath,
            })
            this.requireSuccessfulPluginOperation(await watcher.result)
        } finally {
            watcher?.dispose()
            if (this.activePluginOperations.get(name) === operationId) {
                this.activePluginOperations.delete(name)
            }
        }
    }

    async listInstalledPlugins (): Promise<PluginInfo[]> {
        return this.bridge.invoke('plugins.listInstalled', {})
    }

    async cancelPluginOperation (id: string): Promise<void> {
        await this.bridge.invoke('plugins.cancelOperation', { id })
    }

    override getPluginOperationId (name: string): string|null {
        return this.activePluginOperations.get(name) ?? null
    }

    private async beginPluginOperation (name: string): Promise<string> {
        if (this.activePluginOperations.has(name)) {
            throw new Error(`Plugin operation for ${name} is already running`)
        }
        const operationId = crypto.randomUUID()
        this.activePluginOperations.set(name, operationId)
        try {
            await this.bridge.invoke('plugins.prepareOperation', { id: operationId })
            return operationId
        } catch (error) {
            if (this.activePluginOperations.get(name) === operationId) {
                this.activePluginOperations.delete(name)
            }
            throw error
        }
    }

    private async watchPluginOperation (id: string): Promise<{
        result: Promise<PluginOperation>
        dispose: () => void
    }> {
        let resolveResult: (operation: PluginOperation) => void = () => undefined
        const result = new Promise<PluginOperation>(resolve => {
            resolveResult = resolve
        })
        const dispose = await this.bridge.listen('plugins:operation', operation => {
            if (operation.id === id && operation.status !== 'running') {
                resolveResult(operation)
            }
        })
        return { result, dispose }
    }

    private requireSuccessfulPluginOperation (operation: PluginOperation): void {
        if (operation.status !== 'succeeded') {
            throw new Error(operation.message ?? `Plugin operation ${operation.status}`)
        }
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
            this.bridge.listen('desktop:displayMetricsChanged', () => this.displayMetricsChanged.next()),
            this.bridge.listen('desktop:themeChanged', theme => {
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

class TauriFileUpload extends FileUpload {
    constructor (
        private bridge: HostBridge,
        id: string,
        private name: string,
        private size: number,
    ) {
        super(id)
        this.setTotalSize(size)
    }

    getName (): string { return this.name }
    getMode (): number { return 0o644 }
    getSize (): number { return this.size }

    async read (): Promise<Uint8Array> {
        const data = await this.bridge.invoke('transfer.read', { id: this.id, maxBytes: 256 * 1024 })
        const bytes = Uint8Array.from(data)
        this.setRunning()
        this.increaseProgress(bytes.length)
        if (!bytes.length) {
            this.setCompleted(true)
        }
        return bytes
    }

    close (): void {
        void this.bridge.invoke('transfer.close', { id: this.id })
    }

    override async closeAsync (): Promise<void> {
        await this.bridge.invoke('transfer.close', { id: this.id })
    }

    override cancel (): void {
        this.markCancelled()
        void this.bridge.invoke('transfer.cancel', { id: this.id })
    }
}

class TauriFileDownload extends FileDownload {
    constructor (
        private bridge: HostBridge,
        id: string,
        private name: string,
        private size: number,
    ) {
        super(id)
        this.setTotalSize(size)
    }

    getName (): string { return this.name }
    getSize (): number { return this.size }

    async write (buffer: Uint8Array): Promise<void> {
        await this.bridge.invoke('transfer.write', { id: this.id, data: Array.from(buffer) })
        this.setRunning()
        this.increaseProgress(buffer.length)
        if (this.getCompletedBytes() >= this.size) {
            this.setCompleted(true)
        }
    }

    close (): void {
        void this.bridge.invoke('transfer.close', { id: this.id })
    }

    override async closeAsync (): Promise<void> {
        await this.bridge.invoke('transfer.close', { id: this.id })
    }

    override cancel (): void {
        this.markCancelled()
        void this.bridge.invoke('transfer.cancel', { id: this.id })
    }
}

class TauriDirectoryDownload extends DirectoryDownload {
    constructor (
        private bridge: HostBridge,
        private basePath: string,
        private name: string,
        estimatedSize: number,
    ) {
        super()
        this.setTotalSize(estimatedSize)
    }

    getName (): string { return this.name }
    getSize (): number { return this.getTotalSize() }

    async createDirectory (relativePath: string): Promise<void> {
        await this.bridge.invoke('transfer.createDirectory', {
            baseDirectory: this.basePath,
            relativePath: `${sanitizeTransferName(this.name)}/${sanitizeTransferRelativePath(relativePath)}`,
        })
    }

    async createFile (relativePath: string, mode: number, size: number): Promise<FileDownload> {
        const safePath = `${sanitizeTransferName(this.name)}/${sanitizeTransferRelativePath(relativePath)}`
        const descriptor = await this.bridge.invoke('transfer.openDownload', {
            name: relativePath,
            mode,
            size,
            destination: '',
            baseDirectory: this.basePath,
            relativePath: safePath,
        })
        return new TauriFileDownload(this.bridge, descriptor.id, descriptor.name, size)
    }

    close (): void {
        this.markCancelled()
    }

    override cancel (): void {
        this.markCancelled()
    }
}
