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
    ContextMenuItem,
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
        void this.bridge.invoke('clipboard.writeText', { text: content.text, html: content.html })
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
        if (['macos', 'darwin'].includes(this.runtimeInfo.platform)) {
            this.popupNativeContextMenu(menu)
            return
        }

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

        const opener = document.activeElement as HTMLElement | null
        const interaction = this.renderMenuItems(root, menu)
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
        const stopKeyUp = (keyEvent: KeyboardEvent) => { keyEvent.preventDefault(); keyEvent.stopImmediatePropagation() }
        const closeOnBlur = () => this.closeContextMenu()
        document.addEventListener('mousedown', closeOnOutsideClick, true)
        document.addEventListener('keydown', interaction.keydown, true)
        document.addEventListener('keyup', stopKeyUp, true)
        window.addEventListener('blur', closeOnBlur)
        this.contextMenuElement = root
        this.contextMenuCleanup = () => {
            interaction.dispose()
            document.removeEventListener('mousedown', closeOnOutsideClick, true)
            document.removeEventListener('keydown', interaction.keydown, true)
            document.removeEventListener('keyup', stopKeyUp, true)
            window.removeEventListener('blur', closeOnBlur)
            if (opener?.isConnected) { opener.focus({ preventScroll: true }) }
        }
        root.focus({ preventScroll: true })
    }

    private popupNativeContextMenu (menu: MenuItemOptions[]): void {
        let nextID = 1
        const callbacks = new Map<number, () => void>()
        const serialize = (items: MenuItemOptions[], parentEnabled = true): ContextMenuItem[] => items.map(item => {
            const id = nextID++
            const enabled = parentEnabled && item.enabled !== false
            if (enabled && !item.submenu && item.type !== 'separator' && item.type !== 'submenu' && item.click) {
                callbacks.set(id, item.click)
            }
            return {
                id,
                type: item.submenu ? 'submenu' : item.type ?? 'normal',
                label: item.label ?? '',
                sublabel: item.sublabel,
                enabled,
                checked: !!item.checked,
                submenu: item.submenu ? serialize(item.submenu, enabled) : undefined,
            }
        })
        void this.bridge.invoke('menu.popup', { items: serialize(menu) }).then(({ selectedId }) => {
            const callback = selectedId === null ? undefined : callbacks.get(selectedId)
            if (callback) {
                this.zone.run(callback)
            }
        }).catch(error => console.warn('Could not open native context menu', error))
    }

    private renderMenuItems (root: HTMLElement, items: MenuItemOptions[]): { keydown: (event: KeyboardEvent) => void, dispose: () => void } {
        interface Entry { item: MenuItemOptions, button: HTMLButtonElement, child?: Level, mnemonic?: string, label: string }
        interface Level { element: HTMLElement, entries: Entry[], selected?: Entry, parent?: Level, owner?: Entry }
        let active: Level = { element: root, entries: [] }
        const hover = { timer: undefined as ReturnType<typeof setTimeout> | undefined }
        const cancelHover = () => { clearTimeout(hover.timer); hover.timer = undefined }
        const hideChildren = (level: Level) => {
            for (const entry of level.entries) {
                if (entry.child) {
                    hideChildren(entry.child)
                    entry.child.element.style.display = 'none'
                    entry.button.setAttribute('aria-expanded', 'false')
                }
            }
        }
        const select = (level: Level, entry?: Entry) => {
            cancelHover()
            if (level.selected !== entry) {
                hideChildren(level)
                if (level.selected) { level.selected.button.style.background = 'transparent' }
            }
            level.selected = entry
            active = level
            if (entry) {
                entry.button.style.background = 'var(--bs-primary, #375a9e)'
                entry.button.focus({ preventScroll: true })
            } else {
                level.element.focus({ preventScroll: true })
            }
        }
        const eligible = (level: Level) => level.entries.filter(entry => !entry.button.disabled)
        const openChild = (level: Level, entry: Entry, keyboard = false) => {
            if (entry.button.disabled || !entry.child) { return }
            select(level, entry)
            const submenu = entry.child.element
            submenu.style.display = 'block'
            const rect = entry.button.getBoundingClientRect()
            const width = submenu.offsetWidth
            const height = submenu.offsetHeight
            submenu.style.left = `${Math.max(0, Math.min(rect.right + width > window.innerWidth ? rect.left - width : rect.right, window.innerWidth - width))}px`
            submenu.style.top = `${Math.max(0, Math.min(rect.top, window.innerHeight - height))}px`
            entry.button.setAttribute('aria-expanded', 'true')
            select(entry.child, keyboard ? eligible(entry.child)[0] : undefined)
        }
        const activate = (level: Level, entry?: Entry) => {
            if (!entry || entry.button.disabled) { return }
            if (entry.child) {
                openChild(level, entry, true)
            } else if (entry.item.type !== 'submenu') {
                this.closeContextMenu()
                this.zone.run(() => entry.item.click?.())
            }
        }
        const build = (element: HTMLElement, source: MenuItemOptions[], parent?: Level, owner?: Entry): Level => {
            const level: Level = { element, entries: [], parent, owner }
            element.tabIndex = -1
            const normalized = source.filter((item, index) => item.type !== 'separator' || index > 0 && source[index - 1].type !== 'separator')
            if (normalized[normalized.length - 1]?.type === 'separator') { normalized.pop() }
            const defaultRadios = new Set<MenuItemOptions>()
            let group: MenuItemOptions[] = []
            const finishGroup = () => {
                if (group.length && !group.some(item => item.checked)) { defaultRadios.add(group[0]) }
                group = []
            }
            for (const item of normalized) {
                if (item.type === 'separator') { finishGroup() }
                if (item.type === 'radio') { group.push(item) }
            }
            finishGroup()
            for (const item of normalized) {
                if (item.type === 'separator') {
                    const separator = document.createElement('div')
                    separator.setAttribute('role', 'separator')
                    Object.assign(separator.style, { borderTop: '1px solid var(--bs-border-color, #555)', margin: '4px 0' })
                    element.appendChild(separator)
                    continue
                }
                const row = document.createElement('div')
                row.setAttribute('role', 'none')
                const button = document.createElement('button')
                button.type = 'button'
                button.tabIndex = -1
                button.disabled = item.enabled === false
                const checked = !!item.checked || defaultRadios.has(item)
                const stateful = item.type === 'checkbox' || item.type === 'radio'
                button.setAttribute('role', stateful ? `menuitem${item.type}` : 'menuitem')
                if (stateful) { button.setAttribute('aria-checked', String(checked)) }
                const label = (item.label ?? '').replace(/&&|&/g, value => value === '&&' ? '&' : '')
                const mnemonic = /&([^&])/.exec((item.label ?? '').replace(/&&/g, ''))?.[1].toLocaleLowerCase()
                const entry: Entry = { item, button, label, mnemonic }
                level.entries.push(entry)
                const mark = document.createElement('span')
                mark.textContent = checked ? item.type === 'radio' ? '●' : '✓' : ''
                mark.style.width = '16px'
                mark.style.flexShrink = '0'
                button.appendChild(mark)
                const text = document.createElement('span')
                text.style.display = 'flex'
                text.style.flexDirection = 'column'
                const title = document.createElement('span')
                title.textContent = label
                text.appendChild(title)
                if (item.sublabel) {
                    const subtitle = document.createElement('span')
                    subtitle.textContent = item.sublabel
                    subtitle.style.opacity = '0.65'
                    text.appendChild(subtitle)
                }
                button.appendChild(text)
                Object.assign(button.style, { background: 'transparent', border: '0', color: 'inherit', cursor: button.disabled ? 'default' : 'pointer', display: 'flex', padding: '6px 12px', textAlign: 'left', width: '100%' })
                row.appendChild(button)
                if (item.submenu) {
                    const arrow = document.createElement('span')
                    arrow.textContent = '›'
                    arrow.style.marginLeft = 'auto'
                    button.appendChild(arrow)
                    button.setAttribute('aria-haspopup', 'menu')
                    button.setAttribute('aria-expanded', 'false')
                    const submenu = document.createElement('div')
                    submenu.setAttribute('role', 'menu')
                    Object.assign(submenu.style, { background: 'var(--bs-body-bg, #202124)', border: '1px solid var(--bs-border-color, #555)', borderRadius: '4px', boxShadow: '0 4px 16px rgba(0, 0, 0, .35)', display: 'none', minWidth: '180px', maxWidth: '100vw', maxHeight: '100vh', overflowY: 'auto', padding: '4px 0', position: 'fixed', zIndex: '1' })
                    entry.child = build(submenu, item.submenu, level, entry)
                    row.appendChild(submenu)
                }
                row.addEventListener('mouseenter', () => {
                    if (button.disabled) { cancelHover(); return }
                    select(level, entry)
                    if (entry.child) { hover.timer = setTimeout(() => openChild(level, entry), 400) }
                })
                row.addEventListener('mouseleave', cancelHover)
                button.addEventListener('mousedown', mouseEvent => {
                    mouseEvent.preventDefault()
                    if (entry.child) { openChild(level, entry) } else if (!button.disabled) { select(level, entry) }
                })
                button.addEventListener('click', () => { if (!entry.child) { activate(level, entry) } })
                element.appendChild(row)
            }
            return level
        }
        active = build(root, items)
        const keydown = (event: KeyboardEvent) => {
            event.preventDefault()
            event.stopImmediatePropagation()
            cancelHover()
            const entries = eligible(active)
            const index = entries.indexOf(active.selected!)
            const leave = () => {
                if (active.parent && active.owner) {
                    const { parent, owner } = active
                    hideChildren(parent)
                    select(parent, owner)
                } else { this.closeContextMenu() }
            }
            switch (event.key) {
                case 'ArrowDown': case 'PageDown': select(active, entries[(index + 1) % entries.length]); break
                case 'ArrowUp': case 'PageUp': select(active, entries[(index < 0 ? entries.length : index) - 1] ?? entries[entries.length - 1]); break
                case 'Home': select(active, entries[0]); break
                case 'End': select(active, entries[entries.length - 1]); break
                case 'ArrowRight': if (active.selected) { openChild(active, active.selected, true) } break
                case 'ArrowLeft': if (active.parent) { leave() } break
                case 'Escape': leave(); break
                case 'Enter': activate(active, active.selected); break
                case 'Alt': case 'F10': if (this.runtimeInfo.platform === 'windows') { this.closeContextMenu() } break
                default: {
                    if (event.key.length !== 1 || !event.key.trim() || event.ctrlKey || event.metaKey) { break }
                    const character = event.key.toLocaleLowerCase()
                    const mnemonics = entries.filter(entry => entry.mnemonic === character)
                    const matches = mnemonics.length ? mnemonics : entries.filter(entry => !entry.mnemonic && entry.label.toLocaleLowerCase().startsWith(character))
                    if (!matches.length) { break }
                    const next = matches[(matches.indexOf(active.selected!) + 1) % matches.length]
                    select(active, next)
                    if (matches.length === 1) { activate(active, next) }
                }
            }
        }
        return { keydown, dispose: cancelHover }
    }

    private closeContextMenu (): void {
        this.contextMenuCleanup?.()
        this.contextMenuCleanup = null
        this.contextMenuElement?.remove()
        this.contextMenuElement = null
    }

    async showMessageBox (options: MessageBoxOptions): Promise<MessageBoxResult> {
        return this.bridge.invoke('dialog.message', options)
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
