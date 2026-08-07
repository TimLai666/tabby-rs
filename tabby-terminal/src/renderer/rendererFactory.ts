import { TerminalRenderer, TerminalRendererConstructionOptions } from './terminalRenderer'
import { XtermRenderer } from './xtermRenderer'

export type TerminalRendererFactory = (options: TerminalRendererConstructionOptions) => TerminalRenderer

/**
 * v1 intentionally keeps xterm.js behind this factory. A future Rust-backed
 * renderer can be selected here without changing terminal tab or session APIs.
 */
export const createTerminalRenderer: TerminalRendererFactory = options => new XtermRenderer(options)
