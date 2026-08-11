import { ConnectableTerminalProfile, InputProcessingOptions, StreamProcessingOptions } from 'tabby-terminal'

export interface TauriSerialProfile extends ConnectableTerminalProfile {
    options: TauriSerialProfileOptions
}

export interface TauriSerialProfileOptions extends StreamProcessingOptions {
    port: string|null
    baudRate: number|null
    dataBits: 5|6|7|8
    stopBits: 1|1.5|2
    parity: 'none'|'even'|'odd'|'mark'|'space'
    flowControl: 'none'|'software'|'hardware'
    readTimeoutMs: number
    reconnect: {
        enabled: boolean
        maxAttempts: number
        maxDelayMs: number
    }
    input: InputProcessingOptions
}
