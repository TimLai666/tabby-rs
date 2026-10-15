/** Physical client pixels supplied by the native caption controller. */
export type WindowsCaptionButton = 'minimize' | 'maximize' | 'close'

export interface WindowsCaptionRect {
    x: number
    y: number
    width: number
    height: number
}

export interface WindowsCaptionSnapshot {
    revision: number
    visible: boolean
    scale: number
    layout: {
        clip: WindowsCaptionRect
        buttons: {
            button: WindowsCaptionButton
            bounds: WindowsCaptionRect
            enabled: boolean
        }[]
    }
    pointer: {
        hovered: WindowsCaptionButton | null
        pressed: WindowsCaptionButton | null
    }
    maximized: boolean
    active: boolean
}

export interface WindowsCaptionPaintOptions {
    /** WebView pixels per CSS pixel, including page zoom. */
    devicePixelRatio: number
    /** Configured terminal foreground, independent of the hover base. */
    symbolColor: [number, number, number, number]
    hoverBase: 'black' | 'white'
    windows11: boolean
    /** Already eased progress; omitted entries use the settled pointer state. */
    hoverProgress?: Partial<Record<WindowsCaptionButton, number>>
}
