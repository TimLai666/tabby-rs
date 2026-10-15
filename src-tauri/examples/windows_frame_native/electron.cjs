// Native overlay reference: fixed 14e2d60 app/lib/window.ts, Electron 38.8.6.
// Controlled content and isolated storage; this is not original-app acceptance.
const fs = require('node:fs')
const path = require('node:path')
const { createRequire } = require('node:module')
const { app, nativeTheme } = require('electron')
const appRequire = createRequire(path.resolve(__dirname, '../../../app/package.json'))
const [data, ready, close, mode] = process.argv.slice(2)
if (process.platform !== 'win32' || process.versions.electron !== '38.8.6' || !['native', 'thin', 'full'].includes(mode)) {
    throw new Error('The native reference requires Windows and pinned Electron 38.8.6')
}
fs.mkdirSync(data)
app.setPath('userData', data)
nativeTheme.themeSource = 'dark'
app.whenReady().then(async () => {
    const { BrowserWindow } = appRequire('glasstron')
    const options = {
        width: 800, height: 580, x: 80, y: 60,
        title: 'Isolated pinned Tabby native frame reference',
        minWidth: 400, minHeight: 300,
        maximizable: true, frame: mode === 'native', show: false,
        backgroundColor: '#00000000', acceptFirstMouse: true,
        webPreferences: { nodeIntegration: true, backgroundThrottling: false, contextIsolation: false },
    }
    if (mode !== 'native') {
        options.titleBarStyle = 'hidden'
        options.titleBarOverlay = { color: '#00000000' }
    }
    const window = new BrowserWindow(options)
    window.setMenu(null)
    if (mode !== 'native') window.setTitleBarOverlay({ symbolColor: '#e0e0e0', height: 32 })
    const html = '<!doctype html><html style="margin:0;width:100%;height:100%;background:#1377d1"><body style="margin:0;width:100%;height:100%;background:#1377d1"><div style="position:absolute;right:46px;top:0;width:46px;height:32px;background:repeating-conic-gradient(#e1a42b 0 25%,#6c294c 0 50%) 0/8px 8px"></div></body></html>'
    await window.loadURL(`data:text/html,${encodeURIComponent(html)}`)
    window.setAlwaysOnTop(true)
    window.show()
    window.focus()
    await window.webContents.executeJavaScript('new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))')
    const handle = window.getNativeWindowHandle()
    const hwnd = handle.length === 8 ? handle.readBigUInt64LE().toString() : String(handle.readUInt32LE())
    fs.writeFileSync(ready, JSON.stringify({
        kind: 'tabby-pinned-electron-frame-reference', baseline: '14e2d60b9b6dee84a53c37f05eefeb803787de04',
        electron: process.versions.electron, chromium: process.versions.chrome,
        processId: process.pid, hwnd, mode, bounds: window.getBounds(), contentBounds: window.getContentBounds(),
        controls: mode === 'native' ? 'system' : { color: '#00000000', symbolColor: '#e0e0e0', height: 32 },
    }, null, 2), { flag: 'wx' })
    const timer = setInterval(() => {
        if (fs.existsSync(close)) { clearInterval(timer); window.destroy(); app.quit() }
    }, 30)
    setTimeout(() => app.exit(2), 60000).unref()
}).catch(error => { console.error(error); app.exit(1) })
app.on('window-all-closed', () => app.quit())
