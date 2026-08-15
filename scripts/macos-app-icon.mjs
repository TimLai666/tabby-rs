import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'

function plistString (plist, key) {
    const expression = new RegExp(`<key>${key}</key>\\s*<string>([^<]+)</string>`)
    return plist.match(expression)?.[1] || null
}

export function resolveMacosApplicationIcon (applicationBundle) {
    const plistPath = path.join(applicationBundle, 'Contents', 'Info.plist')
    const resourcesPath = path.join(applicationBundle, 'Contents', 'Resources')
    assert.ok(fs.existsSync(plistPath), `macOS application Info.plist is missing: ${plistPath}`)
    assert.ok(fs.existsSync(resourcesPath), `macOS application Resources directory is missing: ${resourcesPath}`)

    const plist = fs.readFileSync(plistPath, 'utf8')
    const declaredName = plistString(plist, 'CFBundleIconFile') || plistString(plist, 'CFBundleIconName')
    assert.ok(declaredName, `macOS application icon is not declared in ${plistPath}`)

    const filename = declaredName.toLowerCase().endsWith('.icns') ? declaredName : `${declaredName}.icns`
    const iconPath = path.join(resourcesPath, filename)
    assert.ok(fs.existsSync(iconPath), `macOS application icon declared by plist is missing: ${iconPath}`)
    return iconPath
}
