import { Injectable } from '@angular/core'
import { TerminalDecorator, BaseTerminalTabComponent, BaseTerminalProfile, encodeTerminalPath } from 'tabby-terminal'
import { webUtils } from 'electron'
import { ShellType, TerminalTabComponent } from 'tabby-local'

/** @hidden */
@Injectable()
export class PathDropDecorator extends TerminalDecorator {
    attach (terminal: BaseTerminalTabComponent<BaseTerminalProfile>): void {
        setTimeout(() => {
            this.subscribeUntilDetached(terminal, terminal.frontend?.dragOver$.subscribe(event => {
                event.preventDefault()
            }))
            this.subscribeUntilDetached(terminal, terminal.frontend?.drop$.subscribe((event: DragEvent) => {
                for (const file of event.dataTransfer!.files as unknown as Iterable<File>) {
                    this.injectPath(terminal, webUtils.getPathForFile(file))
                }
                event.preventDefault()
            }))
        })
    }

    private injectPath (terminal: BaseTerminalTabComponent<BaseTerminalProfile>, path: string) {
        const shellType = this.getShellType(terminal)
        terminal.sendInput(encodeTerminalPath(
            path,
            shellType,
            terminal.config.store.terminal.bracketedPaste && !!terminal.frontend?.supportsBracketedPaste(),
        ))
    }

    private getShellType (terminal: BaseTerminalTabComponent<BaseTerminalProfile>): ShellType {
        const profileShellType = terminal instanceof TerminalTabComponent ? terminal.profile.options.shellType : null

        return profileShellType ?? 'unix'
    }

}
