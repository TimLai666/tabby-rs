import { Injectable, OnDestroy } from '@angular/core'
import { ConfigService, Platform } from 'tabby-core'

import { HostBridge } from '../api/hostBridge'
import { TauriHostAppService } from './hostApp.service'
import { renderWindowsCaption, WindowCaptionState } from './windowsCaption'

type CaptionButtonKind = 'minimize' | 'maximize' | 'close'
type CaptionButtonStatus = 'normal' | 'hovered' | 'pressed'

interface CaptionAnimation {
    status: CaptionButtonStatus
    progress: number
    from: number
    to: number
    startedAt: number
    duration: number
    animating: boolean
}

/** Own the caption canvas and its scoped native/configuration subscriptions. */
@Injectable()
export class TauriWindowsCaptionService implements OnDestroy {
    private initialization: Promise<void> | null = null
    private destroyed = false
    private active = false
    private hasSnapshot = false
    private eventRevision = 0
    private canvas: HTMLCanvasElement | null = null
    private unlisten: (() => void) | null = null
    private configSubscription: { unsubscribe: () => void } | null = null
    private captionState: WindowCaptionState | null = null
    private animations = new Map<CaptionButtonKind, CaptionAnimation>()
    private animationFrame: number | null = null

    constructor (
        private bridge: HostBridge,
        private config: ConfigService,
        private hostApp: TauriHostAppService,
    ) { }

    initialize (): Promise<void> {
        if (this.initialization) {
            return this.initialization
        }
        if (this.destroyed || this.hostApp.platform !== Platform.Windows) {
            return Promise.resolve()
        }
        this.initialization = this.initializeWindows()
        return this.initialization
    }

    ngOnDestroy (): void {
        this.destroyed = true
        this.cleanup()
    }

    private async initializeWindows (): Promise<void> {
        try {
            await this.config.ready$.toPromise()
            if (this.destroyed) {
                return
            }

            this.active = true
            this.configSubscription = this.config.changed$.subscribe(() => {
                if (this.active && this.hasSnapshot) {
                    this.paint()
                }
            })
            this.createCanvas()

            // Any event delivered while listen() is being acquired or while
            // getState() is pending is newer than this initial read.
            const readRevision = this.eventRevision
            const unlisten = await this.bridge.listen('desktop:windowCaptionChanged', state => {
                if (!this.active) {
                    return
                }
                this.eventRevision++
                this.acceptState(state)
            })
            if (!this.active || this.destroyed) {
                this.release(unlisten)
                return
            }
            this.unlisten = unlisten

            const snapshot = await this.bridge.invoke('window.getState', {})
            if (!this.active || this.destroyed || this.eventRevision !== readRevision) {
                return
            }
            this.acceptState(snapshot?.captionControls ?? null)
        } catch (error) {
            this.cleanup()
            throw error
        }
    }

    private createCanvas (): void {
        const canvas = document.createElement('canvas')
        this.canvas = canvas
        canvas.setAttribute('aria-hidden', 'true')
        canvas.tabIndex = -1
        canvas.style.display = 'none'
        canvas.style.position = 'fixed'
        canvas.style.pointerEvents = 'none'
        canvas.style.zIndex = '2147483647'
        document.body.appendChild(canvas)
    }

    private acceptState (state: WindowCaptionState | null): void {
        if (!this.active) {
            return
        }
        this.hasSnapshot = true
        if (!state) {
            this.captionState = null
            this.animations.clear()
            this.cancelAnimation()
            this.paint()
            return
        }

        const nextState: WindowCaptionState = {
            ...state,
            buttons: state.buttons.map(button => ({ ...button })),
        }
        const now = performance.now()
        const presentKinds = new Set<CaptionButtonKind>()
        for (const button of nextState.buttons) {
            presentKinds.add(button.kind)
            this.updateAnimation(button.kind, button.hovered, button.pressed, now)
        }
        for (const kind of this.animations.keys()) {
            if (!presentKinds.has(kind)) {
                this.animations.delete(kind)
            }
        }
        this.captionState = nextState
        this.paint(now)
    }

    private updateAnimation (
        kind: CaptionButtonKind,
        hovered: boolean,
        pressed: boolean,
        now: number,
    ): void {
        const nextStatus: CaptionButtonStatus = pressed ? 'pressed' : hovered ? 'hovered' : 'normal'
        const animation = this.animations.get(kind)
        if (!animation) {
            const progress = nextStatus === 'hovered' ? 1 : 0
            this.animations.set(kind, {
                status: nextStatus,
                progress,
                from: progress,
                to: progress,
                startedAt: now,
                duration: 0,
                animating: false,
            })
            return
        }

        if (nextStatus === animation.status) {
            this.sampleAnimation(animation, now)
            return
        }

        const previousStatus = animation.status
        const progress = this.sampleAnimation(animation, now)
        animation.status = nextStatus

        if (nextStatus === 'pressed') {
            this.setStableProgress(animation, 0, now)
            return
        }
        if (previousStatus === 'pressed') {
            this.setStableProgress(animation, nextStatus === 'hovered' ? 1 : 0, now)
            return
        }

        const target = nextStatus === 'hovered' ? 1 : 0
        const distance = Math.abs(target - progress)
        if (distance === 0) {
            this.setStableProgress(animation, target, now)
            return
        }
        animation.progress = progress
        animation.from = progress
        animation.to = target
        animation.startedAt = now
        animation.duration = 150 * distance
        animation.animating = true
    }

    private setStableProgress (animation: CaptionAnimation, progress: number, now: number): void {
        animation.progress = progress
        animation.from = progress
        animation.to = progress
        animation.startedAt = now
        animation.duration = 0
        animation.animating = false
    }

    private sampleAnimation (animation: CaptionAnimation, now: number): number {
        if (!animation.animating || animation.duration <= 0) {
            return animation.progress
        }
        const fraction = Math.max(0, Math.min(1, (now - animation.startedAt) / animation.duration))
        const eased = 1 - (1 - fraction) * (1 - fraction)
        animation.progress = animation.from + (animation.to - animation.from) * eased
        if (fraction >= 1) {
            this.setStableProgress(animation, animation.to, now)
        }
        return animation.progress
    }

    private paint (now = performance.now()): void {
        if (!this.active || !this.hasSnapshot || !this.canvas) {
            return
        }
        let renderedState: WindowCaptionState | null = null
        if (this.captionState) {
            renderedState = {
                ...this.captionState,
                buttons: this.captionState.buttons.map(button => {
                    const animation = this.animations.get(button.kind)
                    const progress = animation ? this.sampleAnimation(animation, now) : 0
                    return { ...button, hoverProgress: progress }
                }),
            }
        }
        renderWindowsCaption(
            this.canvas,
            renderedState,
            this.config.store.terminal.colorScheme.foreground,
            this.hostApp.windowsBuild,
        )
        this.scheduleAnimationFrame()
    }

    private scheduleAnimationFrame (): void {
        if (!this.active) {
            return
        }
        if (!this.hasAnimation()) {
            this.cancelAnimation()
            return
        }
        if (this.animationFrame !== null) return
        this.animationFrame = requestAnimationFrame(now => {
            this.animationFrame = null
            if (this.active) {
                this.paint(now)
            }
        })
    }

    private hasAnimation (): boolean {
        for (const animation of this.animations.values()) {
            if (animation.animating) {
                return true
            }
        }
        return false
    }

    private cancelAnimation (): void {
        if (this.animationFrame !== null) {
            cancelAnimationFrame(this.animationFrame)
            this.animationFrame = null
        }
    }

    private release (unlisten: () => void): void {
        try {
            unlisten()
        } catch {
            // Cleanup must not replace the original initialization failure.
        }
    }

    private cleanup (): void {
        this.active = false
        this.cancelAnimation()
        const subscription = this.configSubscription
        this.configSubscription = null
        try {
            subscription?.unsubscribe()
        } catch {
            // Continue releasing the remaining resources on teardown errors.
        }
        const unlisten = this.unlisten
        this.unlisten = null
        if (unlisten) {
            this.release(unlisten)
        }
        const canvas = this.canvas
        this.canvas = null
        if (canvas?.parentNode) {
            try {
                canvas.parentNode.removeChild(canvas)
            } catch {
                // The owned canvas may already have been removed by the host.
            }
        }
        this.captionState = null
        this.hasSnapshot = false
        this.animations.clear()
    }
}
