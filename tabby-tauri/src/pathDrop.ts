import { Injectable } from '@angular/core'
import { Subscription } from 'rxjs'
import { BaseTerminalProfile, BaseTerminalTabComponent, encodeTerminalPath, TerminalDecorator } from 'tabby-terminal'
import { ShellType } from '../../tabby-local/src/api'
import { TerminalTabComponent } from '../../tabby-local/src/components/terminalTab.component'

import { TauriPlatformService } from './services/platform.service'

@Injectable()
export class TauriPathDropDecorator extends TerminalDecorator {
    constructor (private platform: TauriPlatformService) {
        super()
    }

    attach (terminal: BaseTerminalTabComponent<BaseTerminalProfile>): void {
        const subscription = new Subscription()
        this.subscribeUntilDetached(terminal, subscription)

        subscription.add(this.platform.fileDropped$.subscribe(event => {
            const position = this.platform.getFileDropPosition(event)
            const target = document.elementFromPoint(position.x, position.y)
            if (target?.closest('[dropZone]')) { return }
            if (!this.containsPoint(terminal, position.x, position.y)) {
                return
            }

            const shellType = this.getShellType(terminal)
            const bracketedPaste = terminal.config.store.terminal.bracketedPaste && !!terminal.frontend?.supportsBracketedPaste()
            for (const path of event.paths) {
                terminal.sendInput(encodeTerminalPath(path, shellType, bracketedPaste))
            }
        }))
    }

    private containsPoint (terminal: BaseTerminalTabComponent<BaseTerminalProfile>, x: number, y: number): boolean {
        const bounds = terminal.content.nativeElement.getBoundingClientRect()
        return x >= bounds.left && x <= bounds.right && y >= bounds.top && y <= bounds.bottom
    }

    private getShellType (terminal: BaseTerminalTabComponent<BaseTerminalProfile>): ShellType {
        const profileShellType = terminal instanceof TerminalTabComponent ? terminal.profile.options.shellType : null
        return profileShellType ?? 'unix'
    }
}
