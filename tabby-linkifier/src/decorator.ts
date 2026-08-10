import { Subscription } from 'rxjs'
import { Inject, Injectable } from '@angular/core'
import { ConfigService, PlatformService } from 'tabby-core'
import { TerminalDecorator, BaseTerminalTabComponent } from 'tabby-terminal'
import { LinkHandler } from './api'
import { decideUri, UriPolicyContext } from './uriPolicy'

@Injectable()
export class LinkHighlighterDecorator extends TerminalDecorator {
    constructor (
        private config: ConfigService,
        private platform: PlatformService,
        @Inject(LinkHandler) private handlers: LinkHandler[],
    ) {
        super()
    }

    attach (tab: BaseTerminalTabComponent<any>): void {
        const frontend = tab.frontend
        if (!frontend) {
            return
        }

        frontend.setLinkHandler({
            activate: (event, uri) => {
                if (!this.willHandleEvent(event)) {
                    return
                }
                void this.activateUri(uri, tab)
            },
        })

        const openLink = async uri => {
            for (const handler of this.handlers) {
                if (!handler.fullMatchRegex.test(uri)) {
                    continue
                }
                const converted = await handler.convert(uri, tab)
                if (!await handler.verify(converted, tab)) {
                    continue
                }
                await this.activateUri(converted, tab, handler)
                return
            }
        }

        let regex = new RegExp('')
        const regexSource = this.handlers.map(x => `(${x.regex.source})`).join('|')
        try {
            regex = new RegExp(regexSource)
            console.debug('Linkifier regexp', regex)
        } catch (error) {
            console.error('Could not build regex for your link handlers:', error)
            console.error('Regex source was:', regexSource)
            return
        }

        const unregister = frontend.registerLinkProvider({
            regex,
            activate: async (event, uri) => {
                if (!this.willHandleEvent(event)) {
                    return
                }
                await openLink(uri)
            },
        })

        this.subscribeUntilDetached(tab, new Subscription(() => {
            unregister()
            if (tab.frontend === frontend) {
                frontend.setLinkHandler(null)
            }
        }))
    }

    private async activateUri (raw: string, tab: BaseTerminalTabComponent<any>, handler?: LinkHandler): Promise<void> {
        const context: UriPolicyContext = {
            source: 'terminal-output',
            cwd: await tab.session?.getWorkingDirectory() ?? null,
            allowedSchemes: ['http', 'https', 'mailto'],
        }
        const decision = decideUri(raw, context)
        if (decision.action === 'reject') {
            console.debug('Rejected terminal URI:', decision.reason)
            return
        }
        if (decision.action === 'confirm') {
            const result = await this.platform.showMessageBox({
                type: 'warning',
                message: 'Open external link?',
                detail: decision.normalized,
                buttons: ['Open', 'Cancel'],
                defaultId: 1,
                cancelId: 1,
            })
            if (result.response !== 0) {
                return
            }
        }
        if (handler) {
            handler.handle(decision.normalized, tab)
        } else {
            await this.platform.openExternal(decision.normalized)
        }
    }

    private willHandleEvent (event: MouseEvent) {
        const modifier = this.config.store.clickableLinks.modifier
        return !modifier || event[modifier]
    }
}
