'use strict'

// APIs used by TerminalStreamProcessor and cli-spinner. Editing behavior is
// shared with Node 22 in the reference Electron runtime; see readline/README.md.
const { Interface } = require('./readline/interface.cjs')
module.exports = {
    ...require('./readline/callbacks.cjs'),
    Interface,
    createInterface: (...args) => {
        const editor = new Interface(...args)
        // In the reference macOS GUI renderer SIGTSTP is ignored by the OS.
        // PassThrough input has no raw-mode state to change. Keep input usable
        // without asking the WebView's process shim to deliver an OS signal.
        if (process.platform === 'darwin') editor.on('SIGTSTP', () => {})
        return editor
    },
    emitKeypressEvents: require('./readline/emitKeypressEvents.cjs'),
}
