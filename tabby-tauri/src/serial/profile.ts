import { ConnectableTerminalProfile, InputProcessingOptions, LoginScriptsOptions, StreamProcessingOptions } from 'tabby-terminal'

export const BAUD_RATES = [
    110, 150, 300, 1200, 2400, 4800, 9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600, 1500000,
]

export interface TauriSerialProfile extends ConnectableTerminalProfile {
    options: TauriSerialProfileOptions
}

export interface TauriSerialProfileOptions extends StreamProcessingOptions, Partial<LoginScriptsOptions> {
    port: string|null
    baudRate: number|null
    dataBits: 5|6|7|8
    stopBits: 1|1.5|2
    parity: 'none'|'even'|'odd'|'mark'|'space'
    flowControl: 'none'|'software'|'hardware'
    slowSend?: boolean
    readTimeoutMs: number
    reconnect: {
        enabled: boolean
        maxAttempts: number
        maxDelayMs: number
    }
    input: InputProcessingOptions
}
