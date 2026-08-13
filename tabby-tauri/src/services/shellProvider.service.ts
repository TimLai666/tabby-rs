/* eslint-disable @typescript-eslint/no-var-requires */
import { Injectable } from '@angular/core'
import { ConfigService } from 'tabby-core'

import {
    SessionOptions,
    Shell,
    ShellProvider,
} from '../../../tabby-local/src/api'
import { HostBridge } from '../api/hostBridge'
import {
    DetectedShell,
    PreparedSpawnRequest,
} from '../api/shell'
import '../api/windowsIntegration'

function iconFor (icon?: string): string|undefined {
    switch (icon) {
        case 'terminal': return 'fas fa-terminal'
        case 'alpine': return require('../icons/alpine.svg')
        case 'alma': return require('../icons/alma.svg')
        case 'clink': return require('../icons/clink.svg')
        case 'cmd': return require('../icons/cmd.svg')
        case 'cmder': return require('../icons/cmder.svg')
        case 'cmder-powershell': return require('../icons/cmder-powershell.svg')
        case 'cygwin': return require('../icons/cygwin.svg')
        case 'debian': return require('../icons/debian.svg')
        case 'docker': return require('../icons/docker.svg')
        case 'git-bash': return require('../icons/git-bash.svg')
        case 'kali': return require('../icons/kali.svg')
        case 'linux': return require('../icons/linux.svg')
        case 'msys2': return require('../icons/msys2.svg')
        case 'open-euler': return require('../icons/open-euler.svg')
        case 'oracle-linux': return require('../icons/oracle-linux.svg')
        case 'powershell': return require('../icons/powershell.svg')
        case 'powershell-core': return require('../icons/powershell-core.svg')
        case 'suse': return require('../icons/suse.svg')
        case 'ubuntu': return require('../icons/ubuntu.svg')
        case 'vs2017': return require('../icons/vs2017.svg')
        case 'vs2019': return require('../icons/vs2019.svg')
        case 'vs2022': return require('../icons/vs2022.svg')
        default: return undefined
    }
}

function normalizeCompatibilityIDs (shell: DetectedShell): DetectedShell {
    if (shell.providerId === 'windows-stock' && shell.id === 'clink') {
        return { ...shell, id: 'cmd-clink' }
    }
    return shell
}

@Injectable()
export class TauriDetectedShellProvider extends ShellProvider {
    constructor (
        private bridge: HostBridge,
        private config: ConfigService,
    ) {
        super()
    }

    async provide (): Promise<Shell[]> {
        const [result, windows] = await Promise.all([
            this.bridge.invoke('shell.detect', {
                identification: this.config.store?.terminal?.identification ?? null,
            }),
            this.bridge.invoke('windows.integrationStatus', {}),
        ])
        for (const warning of [...result.warnings, ...windows.warnings]) {
            console.warn('[shell detection]', warning)
        }

        const clinkEnabled = this.config.store?.terminal?.enableClink !== false
        const shells = result.shells
            .map(normalizeCompatibilityIDs)
            .filter(shell => clinkEnabled || shell.id !== 'cmd-clink')
        if (clinkEnabled && windows.clinkPath && !shells.some(shell => shell.id === 'cmd-clink')) {
            const cmdIndex = shells.findIndex(shell => shell.id === 'cmd')
            const clink: DetectedShell = {
                id: 'cmd-clink',
                providerId: 'windows-stock',
                name: 'CMD (clink)',
                command: 'cmd.exe',
                args: ['/k', windows.clinkPath, 'inject'],
                env: { WT_SESSION: '0' },
                icon: 'clink',
                shellType: 'cmd',
                hidden: false,
                metadata: { source: 'tauriResource' },
            }
            shells.splice(cmdIndex < 0 ? 0 : cmdIndex, 0, clink)
        }

        return shells.map(shell => this.toShell(shell))
    }

    private toShell (shell: DetectedShell): Shell {
        return {
            id: shell.id,
            name: shell.name,
            command: shell.command,
            args: shell.args,
            env: shell.env,
            fsBase: shell.fsBase,
            cwd: shell.cwd,
            icon: iconFor(shell.icon),
            shellType: shell.shellType,
            hidden: shell.hidden,
        }
    }
}

@Injectable({ providedIn: 'root' })
export class TauriSpawnRequestService {
    constructor (private bridge: HostBridge) { }

    prepare (
        options: SessionOptions,
        runtimeEnvironment: Record<string, string> = {},
    ): Promise<PreparedSpawnRequest> {
        return this.bridge.invoke('shell.prepareSpawn', {
            command: options.command,
            args: options.args,
            cwd: options.cwd,
            profileEnvironment: options.env,
            runtimeEnvironment,
            shellType: options.shellType,
            loginShell: options.args.includes('-l') || options.args.includes('--login'),
        })
    }
}
