import { Subscription } from 'rxjs'
import { Inject, Injectable } from '@angular/core'
import { ConfigService, PlatformService } from 'tabby-core'
import { TerminalDecorator, BaseTerminalTabComponent } from 'tabby-terminal'
import { LinkHandler } from './api'

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
                this.platform.openExternal(uri)
            },
        })

        const openLink = async uri => {
            for (const handler of this.handlers) {
                if (!handler.fullMatchRegex.test(uri)) {
                    continue
                }
                if (!await handler.verify(await handler.convert(uri, tab), tab)) {
                    continue
                }
                handler.handle(await handler.convert(uri, tab), tab)
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

    private willHandleEvent (event: MouseEvent) {
        const modifier = this.config.store.clickableLinks.modifier
        return !modifier || event[modifier]
    }
}
