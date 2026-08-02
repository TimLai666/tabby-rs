import { Injectable } from '@angular/core'

import { SessionOptions, UACService } from '../../../tabby-local/src/api'
import { HostBridge } from '../api/hostBridge'
import '../api/windowsIntegration'

@Injectable()
export class TauriUACService extends UACService {
    override isAvailable = false

    private helperPath: string|null = null

    constructor (private bridge: HostBridge) {
        super()
    }

    async initialize (): Promise<void> {
        const status = await this.bridge.invoke('windows.integrationStatus', {})
        this.helperPath = status.uacHelperPath
        this.isAvailable = status.available && !!this.helperPath
        for (const warning of status.warnings) {
            console.info(`Windows integration: ${warning}`)
        }
    }

    override patchSessionOptionsForUAC (sessionOptions: SessionOptions): SessionOptions {
        if (!this.helperPath || !this.isAvailable) {
            throw new Error('Administrator sessions are unavailable because the UAC helper is missing')
        }

        return {
            ...sessionOptions,
            command: this.helperPath,
            args: [
                '--cwd',
                sessionOptions.cwd ?? '',
                '--',
                sessionOptions.command,
                ...sessionOptions.args,
            ],
        }
    }
}
