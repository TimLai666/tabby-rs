import colors from 'ansi-colors'
import { Injectable } from '@angular/core'
import { TerminalDecorator, BaseTerminalTabComponent, XTermFrontend, SessionMiddleware } from 'tabby-terminal'
import { SSHProfile, SSHTabComponent, PasswordStorageService } from 'tabby-ssh'

import { SudoPromptDetector } from './promptDetector'

const PENDING_PASSWORD_TTL_MS = 30_000

export class AutoSudoPasswordMiddleware extends SessionMiddleware {
    private pendingPasswordToPaste: string|null = null
    private pendingPasswordExpiresAt = 0
    private pendingTimer: ReturnType<typeof setTimeout>|null = null
    private pasteHint = `${colors.black.bgBlackBright(' Tabby ')} ${colors.gray('Press Enter to paste saved password')}`
    private pasteHintLength = colors.stripColor(this.pasteHint).length
    private detector = new SudoPromptDetector()
    private decoder = new TextDecoder('utf-8', { fatal: false })
    private lookupGeneration = 0

    constructor (
        private profile: SSHProfile,
        private ps: PasswordStorageService,
    ) { super() }

    feedFromSession (data: Buffer): void {
        const match = this.detector.feed(this.decoder.decode(data, { stream: true }))
        if (match) {
            const username = match.username ?? this.profile.options.user
            void this.handlePrompt(username)
        }
        this.outputToTerminal.next(data)
    }

    feedFromTerminal (data: Buffer): void {
        if (this.pendingPasswordToPaste) {
            const password = Date.now() <= this.pendingPasswordExpiresAt
                ? this.pendingPasswordToPaste
                : null
            this.clearHint()
            this.cancelPendingPassword()

            if (password && data.length === 1 && data[0] === 13) {
                this.outputToSession.next(Buffer.from(password + '\n'))
                return
            }
        }
        this.outputToSession.next(data)
    }

    async handlePrompt (username: string): Promise<void> {
        const generation = ++this.lookupGeneration
        let password: string|null = null
        try {
            password = await this.loadPassword(username)
        } catch {
            return
        }
        if (generation !== this.lookupGeneration || !password) {
            return
        }

        this.cancelPendingPassword(false)
        this.pendingPasswordToPaste = password
        this.pendingPasswordExpiresAt = Date.now() + PENDING_PASSWORD_TTL_MS
        this.outputToTerminal.next(Buffer.from(this.pasteHint))
        this.pendingTimer = setTimeout(() => {
            if (generation === this.lookupGeneration && this.pendingPasswordToPaste) {
                this.clearHint()
                this.cancelPendingPassword()
            }
        }, PENDING_PASSWORD_TTL_MS)
    }

    async loadPassword (username: string): Promise<string|null> {
        if (this.profile.options.user !== username) {
            return null
        }
        return this.ps.loadPassword(this.profile, username)
    }

    private clearHint (): void {
        const backspaces = Buffer.alloc(this.pasteHintLength, 8)
        const spaces = Buffer.alloc(this.pasteHintLength, 32)
        this.outputToTerminal.next(Buffer.concat([backspaces, spaces, backspaces]))
    }

    private cancelPendingPassword (invalidateLookup = true): void {
        if (invalidateLookup) {
            this.lookupGeneration++
        }
        if (this.pendingTimer) {
            clearTimeout(this.pendingTimer)
            this.pendingTimer = null
        }
        this.pendingPasswordToPaste = null
        this.pendingPasswordExpiresAt = 0
    }
}

@Injectable()
export class AutoSudoPasswordDecorator extends TerminalDecorator {
    private decoratedSessions = new WeakSet()

    constructor (
        private ps: PasswordStorageService,
    ) {
        super()
    }

    private attachToSession (tab: SSHTabComponent) {
        if (!tab.session || this.decoratedSessions.has(tab.session)) {
            return
        }
        this.decoratedSessions.add(tab.session)
        tab.session.middleware.unshift(new AutoSudoPasswordMiddleware(tab.profile, this.ps))
    }

    attach (tab: BaseTerminalTabComponent<any>): void {
        if (!(tab.frontend instanceof XTermFrontend) || !(tab instanceof SSHTabComponent)) {
            return
        }

        setTimeout(() => {
            this.attachToSession(tab)
            this.subscribeUntilDetached(tab, tab.sessionChanged$.subscribe(() => {
                this.attachToSession(tab)
            }))
        })
    }
}
