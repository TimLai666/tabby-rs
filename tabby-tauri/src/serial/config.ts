import { ConfigProvider } from 'tabby-core'

export class TauriSerialConfigProvider extends ConfigProvider {
    defaults = {
        hotkeys: {
            'restart-serial-session': [],
        },
    }

    platformDefaults = {}
}
