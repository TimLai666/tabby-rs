import { Injectable } from '@angular/core'

import {
    PTYInterface,
    PTYProxy,
} from '../../../tabby-local/src/api'
import { TauriSpawnRequestService } from './shellProvider.service'

interface LegacySpawnOptions {
    cwd?: string|null
    env?: Record<string, string>
}

@Injectable()
export class TauriPendingPTYInterface extends PTYInterface {
    constructor (private spawnRequests: TauriSpawnRequestService) {
        super()
    }

    override async spawn (
        command: string,
        args: string[],
        options: LegacySpawnOptions = {},
    ): Promise<PTYProxy> {
        const prepared = await this.spawnRequests.prepare({
            restoreFromPTYID: null,
            command,
            args,
            cwd: options.cwd ?? null,
            env: options.env ?? {},
            width: null,
            height: null,
            shellType: null,
            pauseAfterExit: false,
            runAsAdministrator: false,
        })
        throw new Error(
            `PTY migration is pending in issue #8. Validated executable: ${prepared.executable}`,
        )
    }

    override async restore (_id: string): Promise<PTYProxy|null> {
        return null
    }
}
