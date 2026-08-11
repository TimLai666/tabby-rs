import { ConnectableTerminalProfile, InputProcessingOptions, StreamProcessingOptions } from 'tabby-terminal'

export interface TauriTelnetProfile extends ConnectableTerminalProfile {
    options: TauriTelnetProfileOptions
}

export interface TauriTelnetProfileOptions extends StreamProcessingOptions {
    host: string
    port: number|null
    terminalType: string
    encoding: string
    connectTimeoutMs: number
    keepaliveInterval: number
    keepaliveCountMax: number
    input: InputProcessingOptions
}
