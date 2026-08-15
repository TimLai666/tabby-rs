import { Injectable, Inject } from '@angular/core'
import { TabRecoveryProvider, RecoveryToken } from '../api/tabRecovery'
import { BaseTabComponent, GetRecoveryTokenOptions } from '../components/baseTab.component'
import { Logger, LogService } from './log.service'
import { ConfigService } from './config.service'
import { NewTabParameters } from './tabs.service'
import { createWorkspaceSnapshot, RestorableSessionKind, sanitizeRecoveryToken, validateWorkspaceSnapshot, WorkspaceSnapshot } from '../workspace'

/** @hidden */
@Injectable({ providedIn: 'root' })
export class TabRecoveryService {
    logger: Logger
    enabled = false

    private constructor (
        @Inject(TabRecoveryProvider) private tabRecoveryProviders: TabRecoveryProvider<BaseTabComponent>[]|null,
        private config: ConfigService,
        log: LogService,
    ) {
        this.logger = log.create('tabRecovery')
    }

    async saveTabs (tabs: BaseTabComponent[], activeTab?: BaseTabComponent|null): Promise<void> {
        if (!this.enabled || !this.config.store.recoverTabs) {
            return
        }
        const tokens = (await Promise.all(
            tabs.map(async tab => ({ tab, token: await this.getFullRecoveryToken(tab, { includeState: true }) })),
        )).filter((item): item is { tab: BaseTabComponent, token: RecoveryToken } => !!item.token)
        const restorableTabs = tokens.map(({ tab, token }) => ({
            schemaVersion: 1 as const,
            tabId: tab.tabId,
            title: tab.title,
            customTitle: tab.customTitle,
            profileId: token.profile?.id ?? token.type,
            sessionKind: this.getSessionKind(token.type),
            sessionState: token,
            pinned: tab.pinned,
        }))
        if (!restorableTabs.length) {
            window.localStorage.setItem('tabsRecovery.pending', '[]')
            window.localStorage.setItem('tabsRecovery', '[]')
            window.localStorage.removeItem('tabsRecovery.pending')
            return
        }
        const snapshot = createWorkspaceSnapshot(restorableTabs, activeTab?.tabId)
        const serialized = JSON.stringify(snapshot)
        window.localStorage.setItem('tabsRecovery.pending', serialized)
        window.localStorage.setItem('tabsRecovery', serialized)
        window.localStorage.removeItem('tabsRecovery.pending')
    }

    async getFullRecoveryToken (tab: BaseTabComponent, options?: GetRecoveryTokenOptions): Promise<RecoveryToken|null> {
        const token = await tab.getRecoveryToken(options)
        if (token) {
            token.tabId = tab.tabId
            token.tabTitle = tab.title
            token.tabCustomTitle = tab.customTitle
            token.tabPinned = tab.pinned
            if (tab.icon) {
                token.tabIcon = tab.icon
            }
            if (tab.color) {
                token.tabColor = tab.color
            }
            token.disableDynamicTitle = tab['disableDynamicTitle']
        }
        return token
    }

    async recoverTab (token: RecoveryToken): Promise<NewTabParameters<BaseTabComponent>|null> {
        for (const provider of this.config.enabledServices(this.tabRecoveryProviders ?? [])) {
            try {
                if (!await provider.applicableTo(token)) {
                    continue
                }
                const tab = await provider.recover(token)
                tab.inputs = tab.inputs ?? {}
                tab.inputs.icon = token.tabIcon ?? null
                tab.inputs.color = token.tabColor ?? null
                tab.inputs.title = token.tabTitle || ''
                tab.inputs.customTitle = token.tabCustomTitle || ''
                tab.inputs.pinned = token.tabPinned ?? false
                tab.inputs.disableDynamicTitle = token.disableDynamicTitle
                if (token.tabId) {
                    tab.inputs.tabId = token.tabId
                }
                if (token.__recoveredActive) {
                    tab.inputs.__recoveredActive = true
                }
                return tab
            } catch (error) {
                this.logger.warn('Tab recovery crashed:', token, provider, error)
            }
        }
        return null
    }

    async recoverTabs (): Promise<NewTabParameters<BaseTabComponent>[]> {
        const raw = window.localStorage.tabsRecovery
        if (raw) {
            let tokens: RecoveryToken[] = []
            try {
                const parsed = JSON.parse(raw)
                if (Array.isArray(parsed)) {
                    tokens = parsed.map(token => sanitizeRecoveryToken(token) as RecoveryToken)
                } else {
                    const snapshot = validateWorkspaceSnapshot(parsed)
                    if (snapshot) {
                        tokens = snapshot.tabs
                            .sort((a, b) => this.getLayoutOrder(snapshot, a.tabId) - this.getLayoutOrder(snapshot, b.tabId))
                            .map(tab => ({
                                ...(tab.sessionState as RecoveryToken),
                                tabId: tab.tabId,
                                __recoveredActive: snapshot.activeTabId === tab.tabId,
                            }))
                    }
                }
            } catch (error) {
                this.logger.warn('Saved workspace snapshot is invalid', error)
            }
            const tabs: NewTabParameters<BaseTabComponent>[] = []
            for (const token of tokens) {
                const tab = await this.recoverTab(token)
                if (tab) {
                    tabs.push(tab)
                }
            }
            return tabs
        }
        return []
    }

    private getSessionKind (type: string): RestorableSessionKind {
        if (type.includes('ssh')) {
            return 'ssh'
        }
        if (type.includes('telnet')) {
            return 'telnet'
        }
        if (type.includes('serial')) {
            return 'serial'
        }
        return 'local'
    }

    private getLayoutOrder (snapshot: WorkspaceSnapshot, tabId: string): number {
        const order: string[] = []
        const visit = node => {
            if (node.type === 'pane') {
                order.push(node.tabId)
            } else {
                node.children.forEach(visit)
            }
        }
        visit(snapshot.layout)
        const index = order.indexOf(tabId)
        return index === -1 ? Number.MAX_SAFE_INTEGER : index
    }
}
