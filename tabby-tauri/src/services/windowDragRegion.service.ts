import { Inject, Injectable, OnDestroy } from '@angular/core'

import { HostBridge, RuntimeInfo, TAURI_RUNTIME_INFO } from '../api/hostBridge'

/**
 * Elements that upstream Tabby marks with `-webkit-app-region: drag`.
 * WebKitGTK ignores that CSS property, so Linux custom frames (thin/full)
 * need an explicit native drag request instead.
 */
export const WINDOW_DRAG_REGION_SELECTOR = '.tab-bar > .btn-space.drag, title-bar .title, .preload-logo'

const CLICKABLE_SELECTOR = 'button, a, input, select, textarea, label'

/**
 * Starts a native window move when the pointer presses a drag region on Linux,
 * matching upstream's frameless Electron window. A double press toggles
 * maximization, mirroring Chromium's drag-region behavior on Linux. The
 * shared templates also toggle on `dblclick`; that event is swallowed here so
 * one gesture toggles once (an X11 check showed the pair cancelling out).
 */
@Injectable()
export class TauriWindowDragRegionService implements OnDestroy {
    private listener: ((event: MouseEvent) => void) | null = null
    private doubleClickListener: ((event: MouseEvent) => void) | null = null
    private swallowNextDoubleClick = false

    constructor (
        private bridge: HostBridge,
        @Inject(TAURI_RUNTIME_INFO) private runtimeInfo: RuntimeInfo,
    ) { }

    /**
     * Attaches the document mousedown listener on Linux only. Repeated calls
     * attach one listener. Non-Linux platforms attach nothing: macOS keeps the
     * native transparent title bar and Windows keeps its native caption.
     */
    initialize (): void {
        // Native frames keep GTK's own title bar, so nothing in the page drags the window.
        if (this.listener || this.runtimeInfo.platform !== 'linux' || this.runtimeInfo.startupWindowFrame === 'native') {
            return
        }
        this.listener = event => this.onMouseDown(event)
        this.doubleClickListener = event => this.onDoubleClick(event)
        document.addEventListener('mousedown', this.listener)
        document.addEventListener('dblclick', this.doubleClickListener, true)
    }

    ngOnDestroy (): void {
        if (this.listener) {
            document.removeEventListener('mousedown', this.listener)
            this.listener = null
        }
        if (this.doubleClickListener) {
            document.removeEventListener('dblclick', this.doubleClickListener, true)
            this.doubleClickListener = null
        }
    }

    private dragRegionFor (target: EventTarget | null): Element | null {
        if (!(target instanceof Element)) {
            return null
        }
        const region = target.closest(WINDOW_DRAG_REGION_SELECTOR)
        if (!region) {
            return null
        }
        const clickable = target.closest(CLICKABLE_SELECTOR)
        return clickable && region.contains(clickable) ? null : region
    }

    private onDoubleClick (event: MouseEvent): void {
        // The toggle already happened on the second press; the matching
        // dblclick can land on a different target if the layout moved.
        const handledPress = this.swallowNextDoubleClick
        this.swallowNextDoubleClick = false
        if (event.button === 0 && (handledPress || this.dragRegionFor(event.target))) {
            event.preventDefault()
            event.stopImmediatePropagation()
        }
    }

    private onMouseDown (event: MouseEvent): void {
        if (event.button !== 0 || event.detail !== 1 && event.detail !== 2) {
            return
        }
        if (!this.dragRegionFor(event.target)) {
            return
        }
        event.preventDefault()
        this.swallowNextDoubleClick = event.detail === 2
        const command = event.detail === 2 ? 'window.toggleMaximize' : 'window.startDragging'
        void this.bridge.invoke(command, {}).catch(error => {
            console.warn(`Could not ${event.detail === 2 ? 'toggle maximize' : 'start dragging'} the window`, error)
        })
    }
}
