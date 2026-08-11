import { ConfigProvider } from 'tabby-core'

export class TauriTelnetConfigProvider extends ConfigProvider {
    defaults = {
        hotkeys: {
            'restart-telnet-session': [],
        },
    }

    platformDefaults = {}
}
