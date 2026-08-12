import { InjectionToken } from '@angular/core'
import { BootstrapData, NodeToolchainStatus, PluginInfo, StoredVault, TransferDescriptor } from 'tabby-core'

export type UpdateChannel = 'stable' | 'nightly'

export interface RuntimeInfo {
    host: 'tauri'
    platform: string
    arch: string
    version: string
}

export interface AppIdentity {
    productName: string
    appIdentifier: string
    cliName: string
    urlScheme: string
    dataDirName: string
    credentialService: string
    executable: string
    dataDir: string
    pluginsDir: string
    logsDir: string
    portable: boolean
    portableRoot: string | null
}

export interface PluginOperation {
    id: string
    packageName: string
    action: 'install' | 'uninstall'
    status: 'running' | 'succeeded' | 'failed' | 'cancelled'
    message: string | null
}

export interface PluginDescriptor {
    name: string
    packageName: string
    version: string
    path: string
    entry: string
    isBuiltin: boolean
    isLegacy: boolean
    manifest: Record<string, unknown>
}

export interface PluginSource {
    packageName: string
    entry: string
    code: string
}

export interface PluginBootstrapFailure {
    packageName?: string | null
    phase: string
    message: string
}

export interface CliAliasStatus {
    supported: boolean
    enabled: boolean
    aliasPath: string | null
    conflict: string | null
    message: string | null
}

export interface ConfigReadResult {
    yaml: string
    revision: string | null
    path: string
}

export interface ConfigWriteResult {
    revision: string
    path: string
}

export interface DiagnosticsStatus {
    enabled: boolean
    directory: string
    fileCount: number
    bytes: number
    maxFileBytes: number
    maxFiles: number
    maxBytes: number
    crashMarkerPresent: boolean
}

export interface DiagnosticsAppendRequest {
    level: string
    source: string
    message: string
    fields?: Record<string, unknown>
}

export interface DiagnosticsOptions {
    includeLogs?: boolean
}

export interface DiagnosticsPreviewFile {
    path: string
    size: number
    content: string
    redacted: boolean
}

export interface DiagnosticsPreview {
    schemaVersion: number
    generatedAt: string
    files: DiagnosticsPreviewFile[]
    redactionWarning: string
}

export interface DiagnosticsExportRequest extends DiagnosticsOptions {
    destination: string
}

export interface BackupFile {
    path: string
    sha256: string
    size: number
}

export interface BackupManifest {
    schemaVersion: number
    backupId: string
    createdAt: string
    reason: string
    sourceVersion: string
    channel: UpdateChannel
    files: BackupFile[]
    absent: string[]
}

export interface RestoreReport {
    backupId: string
    restored: string[]
    removed: string[]
}

export interface SecretReference {
    path: string
    kind: string
}

export interface ImportPlan {
    sourceDataDir: string
    config: boolean
    profiles: number
    plugins: string[]
    secretReferences: SecretReference[]
    sourceRevision: string
}

export interface ImportReportItem {
    kind: string
    name: string
    detail: string
}

export interface ImportReport {
    imported: ImportReportItem[]
    skipped: ImportReportItem[]
    failed: ImportReportItem[]
    requiresSecretReentry: string[]
    reportPath: string
    backupId: string
}

export interface VaultStatus {
    unlocked: boolean
    expiresInSeconds: number | null
    secretCount: number
}

export interface VaultSecretSelector {
    type: string
    key: Record<string, unknown>
}

export interface VaultSecretData extends VaultSecretSelector {
    value: string
}

export interface VaultSecretSummary {
    type: string
    key: Record<string, unknown>
}

export interface VaultSummary {
    config: unknown
    secrets: VaultSecretSummary[]
}

export interface VaultSnapshot {
    config: unknown
    secrets: VaultSecretData[]
}

export interface VaultMutationResult {
    stored: StoredVault
    summary: VaultSummary
}

export interface PutVaultFileResult {
    uri: string
    mutation: VaultMutationResult
}

export interface LegacyCliArguments {
    _: string[]
    directory?: string
    command?: string[]
    profileName?: string
    text?: string
    escape: boolean
    providerId?: string
    query?: string
    debug: boolean
    hidden: boolean
    profileNumber?: number
    newWindow: boolean
    safeMode: boolean
    config?: string
}

export interface LaunchRequest {
    profile: string | null
    cwd: string | null
    newWindow: boolean
    safeMode: boolean
    config: string | null
    command: string[]
    urls: string[]
    argv: LegacyCliArguments
}

export interface LaunchContext {
    request: LaunchRequest
    cwd: string
    secondInstance: boolean
    parseError: string | null
}

export interface WindowBounds {
    x: number
    y: number
    width: number
    height: number
}

export interface WindowCapabilities {
    absolutePositioning: boolean
    docking: boolean
    globalHotkey: boolean
    opacity: boolean
    vibrancy: boolean
    progress: boolean
    clipboard: boolean
    dialogs: boolean
    notifications: boolean
}

export interface WindowStateSnapshot {
    visible: boolean
    alwaysOnTop: boolean
    fullscreen: boolean
    maximized: boolean
    minimized: boolean
    focused: boolean
    bounds: WindowBounds
    scaleFactor: number
    capabilities: WindowCapabilities
}

export interface VibrancyOptions {
    enabled: boolean
    effect?: string | null
}

export interface WindowStatePatch {
    visible?: boolean
    alwaysOnTop?: boolean
    fullscreen?: boolean
    maximized?: boolean
    bounds?: WindowBounds
    opacity?: number
    progress?: number | null
    vibrancy?: VibrancyOptions
    colorScheme?: 'system' | 'light' | 'dark'
    title?: string
}

export interface ScreenInfo {
    id: number
    name: string
    primary: boolean
    scaleFactor: number
    bounds: WindowBounds
    workArea: WindowBounds
}

export interface DockingOptions {
    side: 'off' | 'left' | 'right' | 'top' | 'bottom'
    screenId?: number | null
    fill: number
    space: number
    alwaysOnTop: boolean
    minWidth: number
    minHeight: number
}

export interface GlobalHotkeyRegistration {
    id: string
    accelerators: string[]
}

export interface GlobalHotkeyEvent {
    id: string
    accelerator: string
}

export interface OpenDialogOptions {
    multiple: boolean
    directory: boolean
    title?: string | null
}

export interface SaveDialogOptions {
    title?: string | null
    fileName?: string | null
}

export interface TransferDirectoryEntry {
    name: string
    path: string
    directory: boolean
    size: number
    mode: number
    children: TransferDirectoryEntry[]
}

export interface DesktopNotification {
    title: string
    body?: string | null
}

export interface SshConnectRequest {
    profileId: string
    connectionId?: string|null
    host: string
    port: number
    username?: string | null
    auth: SshAuthMethodRef[]
    terminal: SshTerminalRequest
    keepalive?: SshKeepaliveOptions | null
    environment: Record<string, string>
    x11?: boolean
    x11Display?: string|null
    agentForward?: boolean
    jumpChain?: SshJumpRequest[]
}

export interface SshJumpRequest {
    host: string
    port: number
    username?: string|null
    auth: SshAuthMethodRef[]
}

export type SshForwardingType = 'local'|'remote'|'dynamic'

export interface SshForwardingRequest {
    sessionId: string
    kind: SshForwardingType
    bindHost: string
    bindPort: number
    targetAddress: string
    targetPort: number
}

export interface SshForwardingInfo extends SshForwardingRequest {
    id: string
    status: 'starting'|'active'|'stopping'|'stopped'|'failed'
    lastError?: string|null
}

export type SshAuthMethodRef =
    | { type: 'password'; secretRef: string }
    | { type: 'privateKey'; fileRef: string; passphraseRef?: string | null }
    | { type: 'agent'; socket?: string | null }
    | { type: 'keyboardInteractive' }

export interface SshTerminalRequest {
    term: string
    columns: number
    rows: number
    pixelWidth?: number | null
    pixelHeight?: number | null
}

export interface SshKeepaliveOptions {
    intervalMs: number
    maxCount: number
}

export interface SshSessionInfo {
    id: string
    profileId: string
    host: string
    port: number
    username: string
}

export interface TelnetConnectRequest {
    profileId: string
    connectionId?: string|null
    host: string
    port: number
    terminalType: string
    connectTimeoutMs: number
    localEcho: boolean
    keepalive?: TelnetKeepaliveOptions|null
}

export interface TelnetKeepaliveOptions {
    intervalMs: number
    maxCount: number
}

export interface TelnetSessionInfo {
    id: string
    profileId: string
    host: string
    port: number
}

export interface TelnetOutputEvent {
    id: string
    connectionId: string
    profileId: string
    data: number[]
}

export interface TelnetExitEvent {
    id: string
    connectionId: string
    profileId: string
    reason: string
}

export interface TelnetMessageEvent {
    id: string
    connectionId: string
    profileId: string
    message: string
}

export interface TelnetEchoEvent {
    id: string
    connectionId: string
    profileId: string
    forceEcho: boolean
}

export interface SerialPortInfo {
    id: string
    displayName: string
    path: string
    portType: string
    vendorId?: number|null
    productId?: number|null
    serialNumber?: string|null
    manufacturer?: string|null
}

export type SerialParity = 'none'|'even'|'odd'|'mark'|'space'
export type SerialFlowControl = 'none'|'software'|'hardware'

export interface SerialReconnectPolicy {
    enabled: boolean
    maxAttempts: number
    maxDelayMs: number
}

export interface SerialOpenRequest {
    profileId: string
    connectionId: string
    port: string
    baudRate: number
    dataBits: number
    stopBits: number
    parity: SerialParity
    flowControl: SerialFlowControl
    readTimeoutMs: number
    reconnect: SerialReconnectPolicy
}

export interface SerialSessionInfo {
    id: string
    profileId: string
    port: string
    stableId: string
}

export interface SerialConnectionStateEvent {
    id: string
    connectionId: string
    profileId: string
    state: 'connected'|'disconnected'|'reconnecting'|'waiting'|'closed'
    path?: string|null
    error?: string|null
}

export interface SerialOutputEvent {
    id: string
    connectionId: string
    profileId: string
    data: number[]
}

export interface SerialSignalState {
    clearToSend: boolean
    dataSetReady: boolean
}

export interface InstalledFont {
    family: string
    fullName?: string|null
    monospace: boolean|null
    styles: string[]
}

export interface RemoteFileEntry {
    name: string
    fullPath: string
    isDirectory: boolean
    isSymlink: boolean
    mode: number
    size: number
    modified: number | null
    isOperable: boolean
    unoperableReason: string | null
}

export interface SftpSessionInfo {
    id: string
    sshSessionId: string
}

export interface SftpTransferDescriptor {
    id: string
    direction: 'upload'|'download'
    name: string
    size: number | null
    transferred: number
    state: string
}

export interface SshHostKeyPrompt {
    requestId: string
    connectionId: string
    host: string
    port: number
    algorithm: string
    fingerprintSha256: string
    status: 'unknown' | 'changed'
    previousFingerprints: string[]
}

export interface SshAuthPrompt {
    requestId: string
    id: string
    connectionId: string
    name: string
    instructions: string
    prompts: { text: string; echo: boolean }[]
}

export interface SshOutputEvent {
    id: string
    connectionId: string
    profileId: string
    data: number[]
    extended: boolean
}

export interface SshExitEvent {
    id: string
    connectionId: string
    profileId: string
    exitCode: number | null
    signal: string | null
}

export interface SshImportProfile {
    id: string
    name: string
    host: string
    port: number
    user: string | null
    privateKeys: string[]
}

export interface SshImportPreview {
    source: string
    revision: string|null
    profiles: SshImportProfile[]
    conflicts: {
        profileId: string
        profileName: string
        existingProfileId: string
    }[]
}

export interface SshImportReport {
    imported: SshImportProfile[]
    skipped: string[]
    failed: { profileId: string; reason: string }[]
    revision: string
    path: string
}

export interface HostRequestMap {
    'app.bootstrap': {
        request: Record<string, never>
        response: BootstrapData
    }
    'app.runtimeInfo': {
        request: Record<string, never>
        response: RuntimeInfo
    }
    'app.initialLaunch': {
        request: Record<string, never>
        response: LaunchContext | null
    }
    'app.quit': {
        request: Record<string, never>
        response: null
    }
    'backup.create': {
        request: {
            reason: string
            sourceVersion?: string | null
            channel?: UpdateChannel | null
        }
        response: BackupManifest
    }
    'backup.list': {
        request: Record<string, never>
        response: BackupManifest[]
    }
    'backup.restore': {
        request: { backupId: string }
        response: RestoreReport
    }
    'config.read': {
        request: Record<string, never>
        response: ConfigReadResult
    }
    'config.write': {
        request: {
            yaml: string
            expectedRevision?: string | null
            requireMissing?: boolean
        }
        response: ConfigWriteResult
    }
    'diagnostics.status': {
        request: Record<string, never>
        response: DiagnosticsStatus
    }
    'diagnostics.clearLogs': {
        request: Record<string, never>
        response: null
    }
    'diagnostics.append': {
        request: DiagnosticsAppendRequest
        response: null
    }
    'diagnostics.preview': {
        request: DiagnosticsOptions
        response: DiagnosticsPreview
    }
    'diagnostics.export': {
        request: DiagnosticsExportRequest
        response: string
    }
    'identity.get': {
        request: Record<string, never>
        response: AppIdentity
    }
    'identity.aliasStatus': {
        request: Record<string, never>
        response: CliAliasStatus
    }
    'identity.setAlias': {
        request: { enabled: boolean }
        response: CliAliasStatus
    }
    'migration.detect': {
        request: Record<string, never>
        response: ImportPlan[]
    }
    'migration.execute': {
        request: {
            sourceDataDir: string
            config: boolean
            plugins: string[]
        }
        response: ImportReport
    }
    'plugins.nodeStatus': {
        request: { customNodePath?: string | null }
        response: NodeToolchainStatus
    }
    'plugins.install': {
        request: { operationId: string; packageName: string; version: string; customNodePath?: string | null }
        response: PluginOperation
    }
    'plugins.uninstall': {
        request: { operationId: string; packageName: string; customNodePath?: string | null }
        response: PluginOperation
    }
    'plugins.update': {
        request: { operationId: string; packageName: string; version: string; customNodePath?: string | null }
        response: PluginOperation
    }
    'plugins.remove': {
        request: { operationId: string; packageName: string; customNodePath?: string | null }
        response: PluginOperation
    }
    'plugins.listInstalled': {
        request: Record<string, never>
        response: PluginInfo[]
    }
    'plugins.discover': {
        request: Record<string, never>
        response: PluginDescriptor[]
    }
    'plugins.readEntry': {
        request: { packageName: string }
        response: PluginSource
    }
    'plugins.bootstrapPluginStarted': {
        request: { packageName: string }
        response: null
    }
    'plugins.bootstrapPluginCompleted': {
        request: { packageName: string }
        response: null
    }
    'plugins.bootstrapFailed': {
        request: PluginBootstrapFailure
        response: null
    }
    'plugins.bootstrapSucceeded': {
        request: Record<string, never>
        response: null
    }
    'plugins.bootstrapRetry': {
        request: Record<string, never>
        response: null
    }
    'plugins.cancelOperation': {
        request: { id: string }
        response: null
    }
    'vault.status': {
        request: Record<string, never>
        response: VaultStatus
    }
    'vault.unlock': {
        request: {
            stored: StoredVault
            passphrase: string
            rememberForSeconds: number
        }
        response: VaultSummary
    }
    'vault.replace': {
        request: {
            vault: VaultSnapshot
            passphrase: string
            rememberForSeconds: number
        }
        response: VaultMutationResult
    }
    'vault.lock': {
        request: Record<string, never>
        response: null
    }
    'vault.setEnabled': {
        request: {
            enabled: boolean
            passphrase?: string | null
            rememberForSeconds?: number | null
        }
        response: VaultMutationResult | null
    }
    'vault.summary': {
        request: Record<string, never>
        response: VaultSummary
    }
    'vault.snapshot': {
        request: Record<string, never>
        response: VaultSnapshot
    }
    'vault.getSecret': {
        request: { selector: VaultSecretSelector }
        response: string | null
    }
    'vault.putSecret': {
        request: { secret: VaultSecretData }
        response: VaultMutationResult
    }
    'vault.updateSecret': {
        request: {
            selector: VaultSecretSelector
            secret: VaultSecretData
        }
        response: VaultMutationResult
    }
    'vault.removeSecret': {
        request: { selector: VaultSecretSelector }
        response: VaultMutationResult
    }
    'vault.setConfig': {
        request: { config: unknown }
        response: VaultMutationResult
    }
    'vault.putFile': {
        request: { description: string; bytes: number[] }
        response: PutVaultFileResult
    }
    'vault.getFile': {
        request: { id: string }
        response: number[]
    }
    'window.getState': {
        request: Record<string, never>
        response: WindowStateSnapshot
    }
    'window.applyState': {
        request: WindowStatePatch
        response: null
    }
    'window.reload': {
        request: Record<string, never>
        response: null
    }
    'window.minimize': {
        request: Record<string, never>
        response: null
    }
    'window.toggleMaximize': {
        request: Record<string, never>
        response: null
    }
    'window.close': {
        request: Record<string, never>
        response: null
    }
    'window.bringToFront': {
        request: Record<string, never>
        response: null
    }
    'window.openDevtools': {
        request: Record<string, never>
        response: null
    }
    'window.listScreens': {
        request: Record<string, never>
        response: ScreenInfo[]
    }
    'window.setDocking': {
        request: DockingOptions
        response: WindowStateSnapshot
    }
    'window.toggleQuake': {
        request: Record<string, never>
        response: boolean
    }
    'hotkey.replace': {
        request: GlobalHotkeyRegistration
        response: string[]
    }
    'clipboard.readText': {
        request: Record<string, never>
        response: string
    }
    'clipboard.writeText': {
        request: { text: string }
        response: null
    }
    'dialog.open': {
        request: OpenDialogOptions
        response: string[]
    }
    'dialog.save': {
        request: SaveDialogOptions
        response: string | null
    }
    'notification.show': {
        request: DesktopNotification
        response: null
    }
    'desktop.openExternal': {
        request: { url: string }
        response: null
    }
    'desktop.revealPath': {
        request: { path: string }
        response: null
    }
    'desktop.openPath': {
        request: { path: string }
        response: null
    }
    'desktop.readFile': {
        request: { path: string }
        response: number[]
    }
    'transfer.openUpload': {
        request: { paths: string[] }
        response: TransferDescriptor[]
    }
    'transfer.openDownload': {
        request: {
            name: string
            mode: number
            size?: number | null
            destination: string
            baseDirectory?: string | null
            relativePath?: string | null
        }
        response: TransferDescriptor
    }
    'transfer.read': {
        request: { id: string; maxBytes: number }
        response: number[]
    }
    'transfer.write': {
        request: { id: string; data: number[] }
        response: null
    }
    'transfer.close': {
        request: { id: string }
        response: null
    }
    'transfer.cancel': {
        request: { id: string }
        response: null
    }
    'transfer.createDirectory': {
        request: { baseDirectory: string; relativePath: string }
        response: null
    }
    'transfer.listDirectory': {
        request: { path: string }
        response: TransferDirectoryEntry
    }
    'terminal.export': {
        request: { destination: string }
        response: TransferDescriptor
    }
    'ssh.connect': {
        request: SshConnectRequest
        response: SshSessionInfo
    }
    'ssh.hostKeyDecision': {
        request: { requestId: string; decision: 'once' | 'save' | 'reject' }
        response: null
    }
    'ssh.importPreview': {
        request: { path: string; existingProfileIds?: string[] }
        response: SshImportPreview
    }
    'ssh.importApply': {
        request: {
            path: string
            expectedRevision?: string | null
            selections: { profileId: string; action: 'skip' | 'duplicate' | 'overwrite' }[]
        }
        response: SshImportReport
    }
    'ssh.listPrivateKeys': {
        request: Record<string, never>
        response: string[]
    }
    'ssh.authResponse': {
        request: { requestId: string; responses: string[] }
        response: null
    }
    'ssh.write': {
        request: { id: string; data: number[] }
        response: null
    }
    'ssh.resize': {
        request: {
            id: string
            columns: number
            rows: number
            pixelWidth?: number | null
            pixelHeight?: number | null
        }
        response: null
    }
    'ssh.close': {
        request: { id: string }
        response: null
    }
    'ssh.forwardingStart': {
        request: SshForwardingRequest
        response: SshForwardingInfo
    }
    'ssh.forwardingStop': {
        request: { id: string }
        response: null
    }
    'ssh.forwardingList': {
        request: Record<string, never>
        response: SshForwardingInfo[]
    }
    'sftp.open': {
        request: { id: string }
        response: SftpSessionInfo
    }
    'sftp.list': {
        request: { id: string; path: string }
        response: RemoteFileEntry[]
    }
    'sftp.stat': {
        request: { id: string; path: string; follow?: boolean }
        response: RemoteFileEntry
    }
    'sftp.mkdir': {
        request: { id: string; path: string }
        response: null
    }
    'sftp.rename': {
        request: { id: string; from: string; to: string }
        response: null
    }
    'sftp.remove': {
        request: { id: string; path: string; recursive?: boolean }
        response: null
    }
    'sftp.uploadOpen': {
        request: { id: string; path: string; size?: number | null; overwritePolicy?: 'skip'|'overwrite'|'rename' }
        response: SftpTransferDescriptor
    }
    'sftp.upload': {
        request: { id: string; path: string; size?: number | null; overwritePolicy?: 'skip'|'overwrite'|'rename' }
        response: SftpTransferDescriptor
    }
    'sftp.downloadOpen': {
        request: { id: string; path: string }
        response: SftpTransferDescriptor
    }
    'sftp.download': {
        request: { id: string; path: string }
        response: SftpTransferDescriptor
    }
    'sftp.read': {
        request: { id: string; transferId: string; maxBytes: number }
        response: number[]
    }
    'sftp.write': {
        request: { id: string; transferId: string; data: number[] }
        response: SftpTransferDescriptor
    }
    'sftp.closeTransfer': {
        request: { id: string; transferId: string }
        response: SftpTransferDescriptor
    }
    'sftp.cancelTransfer': {
        request: { id: string; transferId: string }
        response: SftpTransferDescriptor
    }
    'sftp.close': {
        request: { id: string }
        response: null
    }
    'telnet.connect': {
        request: TelnetConnectRequest
        response: TelnetSessionInfo
    }
    'telnet.write': {
        request: { id: string; data: number[] }
        response: null
    }
    'telnet.resize': {
        request: { id: string; columns: number; rows: number }
        response: null
    }
    'telnet.close': {
        request: { id: string }
        response: null
    }
    'serial.list': {
        request: Record<string, never>
        response: SerialPortInfo[]
    }
    'serial.open': {
        request: SerialOpenRequest
        response: SerialSessionInfo
    }
    'serial.write': {
        request: { id: string; data: number[] }
        response: null
    }
    'serial.setSignals': {
        request: { id: string; signal: 'requestToSend'|'dataTerminalReady'; value: boolean }
        response: null
    }
    'serial.getSignals': {
        request: { id: string }
        response: SerialSignalState
    }
    'serial.close': {
        request: { id: string }
        response: null
    }
    'font.list': {
        request: Record<string, never>
        response: InstalledFont[]
    }
    'font.refresh': {
        request: Record<string, never>
        response: InstalledFont[]
    }
}

export interface HostEventMap {
    'app.start': BootstrapData
    'app.launch': LaunchContext
    'desktop.hotkey': GlobalHotkeyEvent
    'desktop.windowFocused': boolean
    'desktop.windowMoved': { x: number; y: number }
    'desktop.windowResized': { width: number; height: number }
    'desktop.windowCloseRequested': null
    'desktop.fileDrop': { paths: string[]; x: number; y: number }
    'desktop.themeChanged': 'system' | 'light' | 'dark'
    'desktop.displayMetricsChanged': number
    'transfer.progress': TransferDescriptor
    'ssh.hostKeyPrompt': SshHostKeyPrompt
    'ssh.authPrompt': SshAuthPrompt
    'ssh.output': SshOutputEvent
    'ssh.exit': SshExitEvent
    'ssh.forwardingChanged': SshForwardingInfo
    'telnet.output': TelnetOutputEvent
    'telnet.exit': TelnetExitEvent
    'telnet.message': TelnetMessageEvent
    'telnet.echo': TelnetEchoEvent
    'serial.output': SerialOutputEvent
    'serial.connectionState': SerialConnectionStateEvent
    'serial.portsChanged': SerialPortInfo[]
    'plugins.operation': PluginOperation
}

export abstract class HostBridge {
    abstract invoke<K extends keyof HostRequestMap> (
        command: K,
        request: HostRequestMap[K]['request'],
    ): Promise<HostRequestMap[K]['response']>

    abstract listen<K extends keyof HostEventMap> (
        event: K,
        handler: (payload: HostEventMap[K]) => void,
    ): Promise<() => void>
}

export const TAURI_RUNTIME_INFO = new InjectionToken<RuntimeInfo>('TAURI_RUNTIME_INFO')
