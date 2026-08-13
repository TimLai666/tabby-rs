import { Injectable } from '@angular/core'
import { Subscription } from 'rxjs'
import { BaseTerminalProfile, BaseTerminalTabComponent, encodeTerminalPath, TerminalDecorator } from 'tabby-terminal'
import { ShellType } from '../../tabby-local/src/api'
import { TerminalTabComponent } from '../../tabby-local/src/components/terminalTab.component'

import { HostBridge } from './api/hostBridge'

@Injectable()
export class TauriPathDropDecorator extends TerminalDecorator {
    constructor (private bridge: HostBridge) {
        super()
    }

    attach (terminal: BaseTerminalTabComponent<BaseTerminalProfile>): void {
        const subscription = new Subscription()
        this.subscribeUntilDetached(terminal, subscription)

        void this.bridge.listen('desktop:fileDrop', event => {
            if (!this.containsPoint(terminal, event.x, event.y)) {
                return
            }

            const shellType = this.getShellType(terminal)
            const bracketedPaste = terminal.config.store.terminal.bracketedPaste && !!terminal.frontend?.supportsBracketedPaste()
            for (const path of event.paths) {
                terminal.sendInput(encodeTerminalPath(path, shellType, bracketedPaste))
            }
        }).then(unsubscribe => subscription.add(unsubscribe))
    }

    private containsPoint (terminal: BaseTerminalTabComponent<BaseTerminalProfile>, x: number, y: number): boolean {
        const bounds = terminal.content.nativeElement.getBoundingClientRect()
        const scale = window.devicePixelRatio
        const pointX = x / scale
        const pointY = y / scale
        return pointX >= bounds.left && pointX <= bounds.right && pointY >= bounds.top && pointY <= bounds.bottom
    }

    private getShellType (terminal: BaseTerminalTabComponent<BaseTerminalProfile>): ShellType {
        const profileShellType = terminal instanceof TerminalTabComponent ? terminal.profile.options.shellType : null
        return profileShellType ?? 'unix'
    }
}
