import type { HostBridge } from '../api/hostBridge'

export interface NotificationBridge {
    invoke: HostBridge['invoke']
}

interface NotificationInit {
    body?: string | null
}

type NotificationListener = (event: unknown) => void

/**
 * Builds a stand-in for the web Notification constructor that sends each
 * notification through the native `notification.show` command.
 *
 * Click events are accepted but never fired: tauri-plugin-notification v2 does
 * not deliver desktop click callbacks to the WebView on Linux, so click-to-focus
 * is unverified rather than implemented.
 */
export function createTauriNotification (bridge: NotificationBridge): typeof Notification {
    class TauriNotification {
        static readonly permission = 'granted'

        static requestPermission (): Promise<NotificationPermission> {
            return Promise.resolve('granted')
        }

        readonly body: string
        readonly tag = ''
        onclick: NotificationListener | null = null
        onclose: NotificationListener | null = null
        onerror: NotificationListener | null = null
        onshow: NotificationListener | null = null
        private readonly listeners = new Map<string, Set<NotificationListener>>()

        constructor (public readonly title: string, options: NotificationInit = {}) {
            this.body = options.body ?? ''
            const request = { title, body: options.body ?? null }
            // Failures are logged and reported through the web API's error event
            // rather than thrown: the native call is asynchronous, so a synchronous
            // throw could never carry the real outcome anyway.
            try {
                Promise.resolve(bridge.invoke('notification.show', request))
                    .then(() => this.dispatch('show'))
                    .catch((error: unknown) => {
                        console.warn('Could not show native notification', error)
                        this.dispatch('error', error)
                    })
            } catch (error) {
                console.warn('Could not show native notification', error)
                this.dispatch('error', error)
            }
        }

        private dispatch (type: 'show' | 'error', detail?: unknown): void {
            const event = { type, target: this, detail }
            const handler = type === 'show' ? this.onshow : this.onerror
            for (const listener of [handler, ...this.listeners.get(type) ?? []]) {
                try {
                    listener?.(event)
                } catch (error) {
                    console.warn(`Notification ${type} listener failed`, error)
                }
            }
        }

        addEventListener (type: string, listener: NotificationListener): void {
            let set = this.listeners.get(type)
            if (!set) {
                set = new Set()
                this.listeners.set(type, set)
            }
            set.add(listener)
        }

        removeEventListener (type: string, listener: NotificationListener): void {
            this.listeners.get(type)?.delete(listener)
        }

        close (): void {
            // Native notifications are not tracked after they are sent.
        }
    }
    return TauriNotification as unknown as typeof Notification
}

/** Replaces the global Notification used by shared code such as tabContextMenu. */
export function installTauriNotifications (bridge: NotificationBridge, host: { Notification?: unknown } = window): void {
    host.Notification = createTauriNotification(bridge)
}
