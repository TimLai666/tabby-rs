export type TabProgressStateName = 'none' | 'normal' | 'indeterminate' | 'paused' | 'error'

export type TabProgressSource = 'osc' | 'heuristic' | 'process'

export interface TabProgressState {
    value: number|null
    state: TabProgressStateName
    source: TabProgressSource
}

export interface TabProgressEntry {
    tabId: string
    active: boolean
    progress: TabProgressState
}

export function aggregateTabProgress (entries: TabProgressEntry[]): TabProgressState {
    const active = entries.find(x => x.active && x.progress.state !== 'none')
    if (active) {
        return active.progress
    }

    const attention = entries.find(x => x.progress.state === 'error' || x.progress.state === 'paused')
    if (attention) {
        return attention.progress
    }

    const indeterminate = entries.find(x => x.progress.state === 'indeterminate')
    if (indeterminate) {
        return indeterminate.progress
    }

    let normal: TabProgressEntry|null = null
    for (const entry of entries) {
        if (entry.progress.state !== 'normal' || entry.progress.value === null) {
            continue
        }
        if (!normal || entry.progress.value > (normal.progress.value ?? 0)) {
            normal = entry
        }
    }
    return normal === null ? { value: null, state: 'none', source: 'process' } : normal.progress
}
