// Keyboard behavior follows Electron 38 / Chromium 140.0.7339.41 MenuController:
// https://github.com/chromium/chromium/blob/140.0.7339.41/ui/views/controls/menu/menu_controller.cc
// This fixture checks the production DOM renderer, not native platform geometry.
const provider = window.menuProvider
const opener = document.querySelector('#open-menu')
const chosen = []
const record = value => {
    chosen.push(value)
    document.querySelector('#result').textContent = chosen.join(', ')
}
const items = () => [
    { type: 'separator' },
    { label: 'Rename', commandLabel: 'Rename tab', click: () => record('rename') },
    { label: 'Disabled', enabled: false, submenu: [{ label: 'Blocked', click: () => record('blocked') }] },
    { type: 'separator' }, { type: 'separator' },
    { label: 'Color', sublabel: 'Blue', submenu: [
        { label: 'Red', type: 'radio', click: () => record('red') },
        { label: 'Blue', type: 'radio', checked: true, click: () => record('blue') },
    ] },
    { label: 'Pin', type: 'checkbox', checked: true, click: () => record('pin') },
    { label: 'Close', click: () => record('close') },
    { type: 'separator' },
]
const root = () => document.querySelector('body > [role="menu"]')
const button = text => {
    const buttons = [...root().querySelectorAll('button')]
    return buttons.find(x => x.textContent.trim().replace(/^[●✓]\s*/, '') === text) ?? buttons.find(x => x.textContent.includes(text))
}
const key = value => document.activeElement.dispatchEvent(new KeyboardEvent('keydown', { key: value, bubbles: true, cancelable: true }))
const mouse = (target, type) => target.dispatchEvent(new MouseEvent(type, { bubbles: false }))
const open = (menu = items(), x = innerWidth - 10, y = innerHeight - 10) => {
    opener.focus()
    provider.popupContextMenu(menu, { clientX: x, clientY: y })
}
const assert = (value, message) => { if (!value) throw new Error(message) }
const inside = element => {
    const rect = element.getBoundingClientRect()
    return rect.left >= 0 && rect.top >= 0 && rect.right <= innerWidth + 1 && rect.bottom <= innerHeight + 1
}

opener.addEventListener('click', event => {
    provider.popupContextMenu(items(), event)
    document.querySelector('#result').textContent = chosen.join(', ') || 'No action selected'
})
window.runMenuChecks = async () => {
    const checks = []
    const failures = []
    const test = async (name, run) => {
        try { open(); await run(); checks.push(name) } catch (error) { failures.push(`${name}: ${error.message}`) }
        finally { provider.closeContextMenu() }
    }
    await test('hidden command labels and separator normalization', () => {
        assert(!root().textContent.includes('Rename tab'), 'Command palette label leaked into the menu')
        assert(root().querySelectorAll(':scope > [role="separator"]').length === 1, 'Duplicate or edge separators remain')
    })
    await test('keyboard skips disabled rows and opens submenus', () => {
        key('ArrowDown')
        assert(document.activeElement.textContent.includes('Rename'), 'First Down did not select Rename')
        key('ArrowDown')
        assert(document.activeElement.textContent.includes('Color'), 'Down did not skip disabled/separator rows')
        key('ArrowRight')
        assert(document.activeElement.textContent.includes('Red'), 'Right did not enter submenu')
        key('ArrowDown'); key('Enter')
        assert(chosen.at(-1) === 'blue', 'Submenu action did not run')
        assert(!root(), 'Selected menu remained open')
        assert(document.activeElement === opener, 'Focus did not return to opener')
    })
    await test('left returns to parent and escape dismisses', () => {
        key('ArrowDown'); key('ArrowDown'); key('ArrowRight'); key('ArrowLeft')
        assert(document.activeElement.textContent.includes('Color'), 'Left did not return to parent')
        const count = chosen.length
        key('Escape')
        assert(!root() && chosen.length === count, 'Escape invoked an action or left menu open')
        assert(document.activeElement === opener, 'Escape lost terminal focus')
    })
    await test('home and end select eligible endpoints', () => {
        key('End')
        assert(document.activeElement.textContent.includes('Close'), 'End did not select final action')
        key('Home')
        assert(document.activeElement.textContent.includes('Rename'), 'Home did not select first action')
    })
    await test('disabled submenu cannot expose actions', async () => {
        const row = button('Disabled').parentElement
        mouse(row, 'mouseenter')
        await new Promise(resolve => setTimeout(resolve, 450))
        assert(getComputedStyle(row.querySelector('[role="menu"]')).display === 'none', 'Disabled submenu opened')
    })
    await test('submenu remains inside bottom-right viewport edge', () => {
        const row = button('Color').parentElement
        button('Color').dispatchEvent(new MouseEvent('mousedown', { button: 0, bubbles: true, cancelable: true }))
        const submenu = row.querySelector('[role="menu"]')
        assert(getComputedStyle(submenu).display !== 'none', 'Enabled submenu did not open')
        assert(inside(submenu), 'Submenu clipped outside the viewport')
    })
    await test('accessible checkbox and radio state', () => {
        assert(button('Pin').getAttribute('role') === 'menuitemcheckbox', 'Checkbox role missing')
        assert(button('Pin').getAttribute('aria-checked') === 'true', 'Checkbox checked state missing')
        assert(button('Blue').getAttribute('role') === 'menuitemradio', 'Radio role missing')
    })
    await test('fallback radio selection preserves caller state', () => {
        const menu = [{ label: 'One', type: 'radio' }, { label: 'Between' }, { label: 'Two', type: 'radio' }]
        open(menu)
        assert(button('One').getAttribute('aria-checked') === 'true', 'Empty radio group did not select first item')
        assert(!menu[0].checked, 'Menu rendering mutated caller state')
    })
    await test('opening a second menu cleans up old handlers', () => {
        open([{ label: 'Second', click: () => record('second') }])
        key('ArrowDown'); key('Enter')
        assert(chosen.at(-1) === 'second', 'Second menu did not receive keyboard selection')
        assert(!root(), 'Second menu did not close')
    })
    await test('escape closes only the current submenu', () => {
        key('ArrowDown'); key('ArrowDown'); key('ArrowRight'); key('Escape')
        assert(root() && document.activeElement.textContent.includes('Color'), 'Submenu Escape closed the entire menu')
        key('Escape')
        assert(!root(), 'Root Escape did not dismiss')
    })
    await test('character matching cycles duplicates and invokes a unique match', () => {
        key('c')
        assert(root() && document.activeElement.textContent.includes('Color'), 'First matching item not highlighted')
        key('c')
        assert(root() && document.activeElement.textContent.includes('Close'), 'Repeated character did not cycle')
        key('p')
        assert(!root() && chosen.at(-1) === 'pin', 'Unique matching item did not execute')
    })
    await test('tab and space do not select or escape the menu', () => {
        key('ArrowDown'); key('Tab'); key(' ')
        assert(root() && document.activeElement.textContent.includes('Rename'), 'Tab/Space changed the menu selection')
    })
    await test('secondary label appears below the main label', () => {
        const color = button('Color')
        const subtitle = [...color.querySelectorAll('span')].find(element => element.textContent === 'Blue')
        assert(subtitle, 'Secondary label missing')
        const text = [...color.querySelectorAll('span')].find(element => element.textContent === 'Color')
        assert(text && subtitle.getBoundingClientRect().top >= text.getBoundingClientRect().bottom - 1, 'Secondary label is not on the second line')
    })
    await test('outside click dismisses without running an action', () => {
        const count = chosen.length
        document.querySelector('h1').dispatchEvent(new MouseEvent('mousedown', { bubbles: true }))
        assert(!root() && chosen.length === count, 'Outside click failed to cancel')
    })
    await test('window blur dismisses without running an action', () => {
        const count = chosen.length
        window.dispatchEvent(new Event('blur'))
        assert(!root() && chosen.length === count, 'Window blur failed to cancel')
    })
    await test('dismissal cancels pending submenu hover', async () => {
        const row = button('Color').parentElement
        mouse(row, 'mouseenter')
        key('Escape')
        await new Promise(resolve => setTimeout(resolve, 450))
        assert(!root() && document.activeElement === opener, 'Cancelled hover reopened or stole focus')
    })
    await test('explicit mnemonic wins over plain first-letter matches', () => {
        open([{ label: 'Open', click: () => record('plain-o') }, { label: 'C&opy', click: () => record('mnemonic-o') }])
        key('o')
        assert(!root() && chosen.at(-1) === 'mnemonic-o', 'Mnemonic priority did not match Chromium')
    })
    await test('explicit mnemonic excludes alternate first-letter activation', () => {
        open([{ label: 'C&opy', click: () => record('copy') }])
        const count = chosen.length
        key('c')
        assert(root() && chosen.length === count, 'First letter activated a different explicit mnemonic')
    })
    await test('escape closes a mouse-opened submenu before the root', () => {
        button('Color').dispatchEvent(new MouseEvent('mousedown', { button: 0, bubbles: true, cancelable: true }))
        key('Escape')
        assert(root() && document.activeElement.textContent.includes('Color'), 'Mouse-opened submenu Escape dismissed the root')
    })
    await test('keyboard navigation enters a mouse-opened submenu', () => {
        button('Color').dispatchEvent(new MouseEvent('mousedown', { button: 0, bubbles: true, cancelable: true }))
        key('ArrowDown')
        assert(document.activeElement === button('Red'), 'Down skipped the mouse-opened submenu')
    })
    await test('menu keystrokes do not reach application shortcuts', () => {
        let received = false
        const listener = () => { received = true }
        document.addEventListener('keydown', listener)
        try { key('ArrowDown'); assert(!received, 'Menu keystroke reached application handler') }
        finally { document.removeEventListener('keydown', listener) }
    })
    return { ok: failures.length === 0, checks, failures }
}
