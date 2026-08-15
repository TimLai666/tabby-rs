export const WORKSPACE_SCHEMA_VERSION = 1
export const MIN_WORKSPACE_RATIO = 0.05

export type WorkspaceDirection = 'horizontal' | 'vertical'

export interface SplitNode {
    type: 'split'
    direction: WorkspaceDirection
    ratios: number[]
    children: WorkspaceNode[]
}

export interface PaneNode {
    type: 'pane'
    tabId: string
}

export type WorkspaceNode = SplitNode | PaneNode

export type RestorableSessionKind = 'local' | 'ssh' | 'telnet' | 'serial'

export interface RestorableTab {
    [_: string]: any
    schemaVersion: 1
    tabId: string
    title?: string
    customTitle?: string
    profileId: string
    sessionKind: RestorableSessionKind
    sessionState: unknown
    pinned?: boolean
}

export interface WorkspaceSnapshot {
    schemaVersion: 1
    activeTabId?: string
    tabs: RestorableTab[]
    layout: WorkspaceNode
}

export type WorkspaceAction =
    | { type: 'focus', tabId: string }
    | { type: 'resize', path: number[], index: number, delta: number }
    | { type: 'reorder', parentPath: number[], from: number, to: number }
    | { type: 'split', tabId: string, direction: WorkspaceDirection, tab: RestorableTab }
    | { type: 'move', tabId: string, targetTabId: string, direction: WorkspaceDirection, before?: boolean }
    | { type: 'unsplit', tabId: string }
    | { type: 'remove', tabId: string }

function isRecord (value: unknown): value is Record<string, any> {
    return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function equalize (count: number): number[] {
    return count > 0 ? Array.from({ length: count }, () => 1 / count) : []
}

export function normalizeRatios (ratios: unknown, childCount: number): number[] {
    if (childCount <= 0) {
        return []
    }

    const values = Array.isArray(ratios)
        ? ratios.slice(0, childCount).map(value => typeof value === 'number' && Number.isFinite(value) ? Math.max(0, value) : 0)
        : []
    while (values.length < childCount) {
        values.push(0)
    }

    const total = values.reduce((sum, value) => sum + value, 0)
    if (total <= 0) {
        return equalize(childCount)
    }

    const normalized = values.map(value => value / total)
    for (let i = 0; i < normalized.length; i++) {
        if (normalized[i] >= MIN_WORKSPACE_RATIO) {
            continue
        }
        const donor = normalized.findIndex((value, index) => index !== i && value > MIN_WORKSPACE_RATIO)
        if (donor === -1) {
            return equalize(childCount)
        }
        const amount = Math.min(MIN_WORKSPACE_RATIO - normalized[i], normalized[donor] - MIN_WORKSPACE_RATIO)
        normalized[i] += amount
        normalized[donor] -= amount
    }

    const normalizedTotal = normalized.reduce((sum, value) => sum + value, 0)
    return normalized.map(value => value / normalizedTotal)
}

function normalizeNode (node: unknown, seenTabs: Set<string>): WorkspaceNode | null {
    if (!isRecord(node) || node.type !== 'pane' && node.type !== 'split') {
        return null
    }

    if (node.type === 'pane') {
        if (typeof node.tabId !== 'string' || !node.tabId || seenTabs.has(node.tabId)) {
            return null
        }
        seenTabs.add(node.tabId)
        return { type: 'pane', tabId: node.tabId }
    }

    if (node.direction !== 'horizontal' && node.direction !== 'vertical' || !Array.isArray(node.children)) {
        return null
    }
    const children: WorkspaceNode[] = []
    for (const child of node.children) {
        const normalized = normalizeNode(child, seenTabs)
        if (!normalized) {
            return null
        }
        children.push(normalized)
    }
    if (!children.length) {
        return null
    }
    return {
        type: 'split',
        direction: node.direction,
        ratios: normalizeRatios(node.ratios, children.length),
        children,
    }
}

export function sanitizeRecoveryToken (value: unknown): unknown {
    if (Array.isArray(value)) {
        return value.map(sanitizeRecoveryToken)
    }
    if (!isRecord(value)) {
        return value
    }
    const blocked = new Set(['clipboard', 'commandhistory', 'history', 'passphrase', 'password', 'privatekey', 'privatekeys', 'scrollback', 'secret', 'token'])
    const result: Record<string, any> = {}
    for (const [key, child] of Object.entries(value)) {
        if (blocked.has(key.toLowerCase())) {
            continue
        }
        result[key] = sanitizeRecoveryToken(child)
    }
    return result
}

export function validateWorkspaceSnapshot (value: unknown): WorkspaceSnapshot | null {
    if (!isRecord(value) || value.schemaVersion !== WORKSPACE_SCHEMA_VERSION || !Array.isArray(value.tabs)) {
        return null
    }

    const tabs: RestorableTab[] = []
    const tabIds = new Set<string>()
    for (const tab of value.tabs) {
        if (!isRecord(tab) || tab.schemaVersion !== WORKSPACE_SCHEMA_VERSION || typeof tab.tabId !== 'string' || !tab.tabId || tabIds.has(tab.tabId)) {
            return null
        }
        if (typeof tab.profileId !== 'string' || !tab.profileId || !['local', 'ssh', 'telnet', 'serial'].includes(tab.sessionKind)) {
            return null
        }
        tabIds.add(tab.tabId)
        tabs.push({
            ...tab,
            schemaVersion: 1,
            tabId: tab.tabId,
            title: typeof tab.title === 'string' ? tab.title : undefined,
            customTitle: typeof tab.customTitle === 'string' ? tab.customTitle : undefined,
            profileId: tab.profileId,
            sessionKind: tab.sessionKind,
            sessionState: sanitizeRecoveryToken(tab.sessionState),
            pinned: typeof tab.pinned === 'boolean' ? tab.pinned : undefined,
        })
    }

    const seenTabs = new Set<string>()
    const layout = normalizeNode(value.layout, seenTabs)
    if (!layout || seenTabs.size !== tabIds.size || [...seenTabs].some(tabId => !tabIds.has(tabId))) {
        return null
    }
    if (value.activeTabId !== undefined && (typeof value.activeTabId !== 'string' || !tabIds.has(value.activeTabId))) {
        return null
    }
    return {
        schemaVersion: 1,
        activeTabId: value.activeTabId,
        tabs,
        layout,
    }
}

export function createWorkspaceSnapshot (tabs: RestorableTab[], activeTabId?: string): WorkspaceSnapshot {
    const safeTabs = tabs.map(tab => ({
        ...tab,
        schemaVersion: 1 as const,
        sessionState: sanitizeRecoveryToken(tab.sessionState),
    }))
    const layout: WorkspaceNode = safeTabs.length === 1
        ? { type: 'pane', tabId: safeTabs[0].tabId }
        : {
            type: 'split',
            direction: 'horizontal',
            ratios: normalizeRatios([], safeTabs.length),
            children: safeTabs.map(tab => ({ type: 'pane', tabId: tab.tabId })),
        }
    return validateWorkspaceSnapshot({
        schemaVersion: 1,
        activeTabId,
        tabs: safeTabs,
        layout,
    })!
}

function getNode (root: WorkspaceNode, path: number[]): WorkspaceNode | null {
    let current = root
    for (const index of path) {
        if (current.type !== 'split' || !current.children[index]) {
            return null
        }
        current = current.children[index]
    }
    return current
}

function removePane (root: WorkspaceNode, tabId: string): WorkspaceNode | null {
    if (root.type === 'pane') {
        return root.tabId === tabId ? null : root
    }
    const children: WorkspaceNode[] = []
    for (const child of root.children) {
        const next = removePane(child, tabId)
        if (next) {
            children.push(next)
        }
    }
    if (!children.length) {
        return null
    }
    if (children.length === 1) {
        return children[0]
    }
    return { ...root, children, ratios: normalizeRatios(root.ratios, children.length) }
}

function findPane (root: WorkspaceNode, tabId: string): boolean {
    if (root.type === 'pane') {
        return root.tabId === tabId
    }
    return root.children.some(child => findPane(child, tabId))
}

function replaceNode (root: WorkspaceNode, path: number[], replacement: WorkspaceNode): WorkspaceNode {
    if (!path.length) {
        return replacement
    }
    if (root.type !== 'split') {
        return root
    }
    const [index, ...rest] = path
    if (!root.children[index]) {
        return root
    }
    const children = [...root.children]
    children[index] = replaceNode(children[index], rest, replacement)
    return { ...root, children }
}

function replacePane (root: WorkspaceNode, tabId: string, replacement: WorkspaceNode): WorkspaceNode {
    if (root.type === 'pane') {
        return root.tabId === tabId ? replacement : root
    }
    return { ...root, children: root.children.map(child => replacePane(child, tabId, replacement)) }
}

function insertBeside (root: WorkspaceNode, targetTabId: string, moved: WorkspaceNode, direction: WorkspaceDirection, before: boolean): WorkspaceNode {
    if (root.type === 'pane') {
        if (root.tabId !== targetTabId) {
            return root
        }
        return {
            type: 'split',
            direction,
            ratios: [0.5, 0.5],
            children: before ? [moved, root] : [root, moved],
        }
    }
    return { ...root, children: root.children.map(child => insertBeside(child, targetTabId, moved, direction, before)) }
}

export function workspaceReducer (snapshot: WorkspaceSnapshot, action: WorkspaceAction): WorkspaceSnapshot {
    const current = validateWorkspaceSnapshot(snapshot)
    if (!current) {
        return snapshot
    }

    if (action.type === 'focus') {
        return findPane(current.layout, action.tabId) ? { ...current, activeTabId: action.tabId } : current
    }

    if (action.type === 'resize') {
        const node = getNode(current.layout, action.path)
        if (!node || node.type !== 'split' || action.index < 0 || action.index >= node.children.length - 1 || !Number.isFinite(action.delta)) {
            return current
        }
        const ratios = [...node.ratios]
        const total = ratios[action.index] + ratios[action.index + 1]
        const next = Math.max(MIN_WORKSPACE_RATIO, Math.min(total - MIN_WORKSPACE_RATIO, ratios[action.index] + action.delta))
        ratios[action.index] = next
        ratios[action.index + 1] = total - next
        const replacement = { ...node, ratios: normalizeRatios(ratios, node.children.length) }
        return { ...current, layout: replaceNode(current.layout, action.path, replacement) }
    }

    if (action.type === 'reorder') {
        const node = getNode(current.layout, action.parentPath)
        if (!node || node.type !== 'split' || action.from < 0 || action.from >= node.children.length || action.to < 0 || action.to >= node.children.length) {
            return current
        }
        const children = [...node.children]
        const [child] = children.splice(action.from, 1)
        children.splice(action.to, 0, child)
        return { ...current, layout: replaceNode(current.layout, action.parentPath, { ...node, children, ratios: normalizeRatios(node.ratios, children.length) }) }
    }

    if (action.type === 'split') {
        if (current.tabs.some(tab => tab.tabId === action.tab.tabId) || !findPane(current.layout, action.tabId)) {
            return current
        }
        const split: SplitNode = {
            type: 'split',
            direction: action.direction,
            ratios: [0.5, 0.5],
            children: [{ type: 'pane', tabId: action.tabId }, { type: 'pane', tabId: action.tab.tabId }],
        }
        return {
            ...current,
            tabs: [...current.tabs, action.tab],
            layout: replacePane(current.layout, action.tabId, split),
        }
    }

    if (action.type === 'move') {
        if (action.tabId === action.targetTabId || !findPane(current.layout, action.tabId) || !findPane(current.layout, action.targetTabId)) {
            return current
        }
        const layoutWithoutSource = removePane(current.layout, action.tabId)
        if (!layoutWithoutSource) {
            return current
        }
        return {
            ...current,
            layout: insertBeside(layoutWithoutSource, action.targetTabId, { type: 'pane', tabId: action.tabId }, action.direction, action.before ?? false),
        }
    }

    if (action.type === 'unsplit') {
        if (!findPane(current.layout, action.tabId) || current.layout.type === 'pane') {
            return current
        }
        const layoutWithoutPane = removePane(current.layout, action.tabId)
        if (!layoutWithoutPane) {
            return current
        }
        return {
            ...current,
            layout: {
                type: 'split',
                direction: 'horizontal',
                ratios: [0.5, 0.5],
                children: [layoutWithoutPane, { type: 'pane', tabId: action.tabId }],
            },
        }
    }

    if (!findPane(current.layout, action.tabId)) {
        return current
    }
    const layout = removePane(current.layout, action.tabId)
    const tabs = current.tabs.filter(tab => tab.tabId !== action.tabId)
    if (!layout || !tabs.length) {
        return current
    }
    return { ...current, activeTabId: current.activeTabId === action.tabId ? tabs[0].tabId : current.activeTabId, tabs, layout }
}
