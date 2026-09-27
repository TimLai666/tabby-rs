import { ConnectableTerminalProfile, InputProcessingOptions, LoginScriptsOptions, StreamProcessingOptions } from 'tabby-terminal'

export interface TauriTelnetProfile extends ConnectableTerminalProfile {
    options: TauriTelnetProfileOptions
}

export interface TauriTelnetProfileOptions extends StreamProcessingOptions, LoginScriptsOptions {
    host: string
    port: number|null
    terminalType: string
    encoding: string
    connectTimeoutMs: number
    keepaliveInterval: number
    keepaliveCountMax: number
    input: InputProcessingOptions
}
