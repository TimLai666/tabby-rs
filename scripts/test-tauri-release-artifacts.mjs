import assert from 'node:assert/strict'
import crypto from 'node:crypto'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { execFileSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const checker = path.join(root, 'scripts/assert-tauri-release-artifacts.mjs')
const stager = path.join(root, 'scripts/stage-tauri-release.mjs')

function run (staging, platform, bundles, environmentOnly = false) {
    const argumentsList = [checker, '--platform', platform, '--bundles', bundles]
    if (!environmentOnly) argumentsList.splice(1, 0, staging)
    return execFileSync(process.execPath, argumentsList, {
        cwd: root,
        encoding: 'utf8',
        stdio: ['ignore', 'pipe', 'pipe'],
        env: environmentOnly ? { ...process.env, TABBY_RS_RELEASE_STAGING: staging } : process.env,
    }).trim()
}

const linux = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-rs-linux-release-'))
const linuxArtifact = path.join(linux, 'tabby-rs_1.0.231-tabbyrs.1_amd64.AppImage')
const linuxSignature = `${linuxArtifact}.sig`
fs.writeFileSync(linuxArtifact, 'appimage')
fs.writeFileSync(linuxSignature, 'signature')
fs.writeFileSync(path.join(linux, 'tabby-rs.deb'), 'deb')
fs.writeFileSync(path.join(linux, 'tabby-rs.rpm'), 'rpm')
fs.writeFileSync(path.join(linux, 'tabby-rs-metadata.json'), JSON.stringify({
    version: '1.0.231-tabbyrs.1',
    platform: 'linux',
    arch: 'x86_64',
}))
assert.equal(run(linux, 'linux', 'appimage,deb,rpm'), linuxArtifact)
assert.equal(run(linux, 'linux', 'appimage,deb,rpm', true), linuxArtifact)
fs.writeFileSync(path.join(linux, 'update-manifest.json'), JSON.stringify({
    version: '1.0.231-tabbyrs.1',
    platform: 'linux',
    arch: 'x86_64',
    url: `https://updates.example.test/${path.basename(linuxArtifact)}`,
    sha256: crypto.createHash('sha256').update(fs.readFileSync(linuxArtifact)).digest('hex'),
    size: fs.statSync(linuxArtifact).size,
    signature: 'signature',
}))
assert.equal(run(linux, 'linux', 'appimage,deb,rpm'), linuxArtifact)
const manifest = JSON.parse(fs.readFileSync(path.join(linux, 'update-manifest.json'), 'utf8'))
manifest.size++
fs.writeFileSync(path.join(linux, 'update-manifest.json'), JSON.stringify(manifest))
assert.throws(() => run(linux, 'linux', 'appimage,deb,rpm'), /size does not match primary artifact/)
fs.rmSync(path.join(linux, 'update-manifest.json'))
fs.rmSync(linuxArtifact)
fs.rmSync(linuxSignature)
fs.writeFileSync(path.join(linux, 'tabby-rs.AppImage'), 'appimage')
fs.writeFileSync(path.join(linux, 'tabby-rs.AppImage.sig'), 'signature')
assert.throws(() => run(linux, 'linux', 'appimage,deb,rpm'), /does not contain release version/)
fs.rmSync(path.join(linux, 'tabby-rs-metadata.json'))
fs.rmSync(path.join(linux, 'tabby-rs.rpm'))
assert.throws(() => run(linux, 'linux', 'appimage,deb,rpm'), /missing rpm artifact/)

const windows = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-rs-windows-release-'))
fs.writeFileSync(path.join(windows, 'tabby-rs-setup.exe'), 'installer')
fs.writeFileSync(path.join(windows, 'tabby-rs-setup.exe.sig'), 'signature')
assert.equal(run(windows, 'windows', 'nsis'), path.join(windows, 'tabby-rs-setup.exe'))

const macos = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-rs-macos-release-'))
fs.writeFileSync(path.join(macos, 'tabby-rs.dmg'), 'dmg')
fs.writeFileSync(path.join(macos, 'tabby-rs.app.tar.gz'), 'updater')
fs.writeFileSync(path.join(macos, 'tabby-rs.app.tar.gz.sig'), 'signature')
const macosDmgIcon = path.join(macos, 'dmg', 'icon.icns')
fs.mkdirSync(path.dirname(macosDmgIcon), { recursive: true })
fs.copyFileSync(path.join(root, 'build/mac/icon.icns'), macosDmgIcon)
const macosApplication = path.join(macos, 'Tabby RS.app')
const macosInfo = path.join(macosApplication, 'Contents', 'Info.plist')
const macosIcon = path.join(macosApplication, 'Contents', 'Resources', 'Tabby RS.icns')
fs.mkdirSync(path.dirname(macosIcon), { recursive: true })
fs.copyFileSync(path.join(root, 'build/mac/icon.icns'), macosIcon)
fs.writeFileSync(macosInfo, `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict><key>CFBundleIconFile</key><string>Tabby RS.icns</string></dict></plist>
`)
fs.writeFileSync(path.join(macos, 'tabby-rs-metadata.json'), JSON.stringify({
    version: '1.0.231-tabbyrs.1',
    platform: 'macos',
    arch: 'aarch64',
}))
assert.throws(() => run(macos, 'macos', 'dmg'), /must request the app bundle/)
assert.equal(run(macos, 'macos', 'app,dmg'), path.join(macos, 'tabby-rs.app.tar.gz'))
fs.writeFileSync(macosInfo, `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict><key>CFBundleIconFile</key><string>icon.icns</string></dict></plist>
`)
assert.throws(() => run(macos, 'macos', 'app,dmg'), /icon declared by plist is missing/)
fs.writeFileSync(macosInfo, `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict><key>CFBundleIconFile</key><string>Tabby RS.icns</string></dict></plist>
`)
fs.writeFileSync(macosDmgIcon, 'stale DMG icon')
assert.throws(() => run(macos, 'macos', 'app,dmg'), /DMG icon does not match build\/mac\/icon\.icns/)
fs.copyFileSync(path.join(root, 'build/mac/icon.icns'), macosDmgIcon)
fs.writeFileSync(macosIcon, 'stale icon')
assert.throws(() => run(macos, 'macos', 'app,dmg'), /does not match build\/mac\/icon\.icns/)

const bundle = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-rs-bundle-'))
const bundleDmg = path.join(bundle, 'dmg')
const bundleMacos = path.join(bundle, 'macos')
fs.mkdirSync(bundleDmg)
fs.mkdirSync(bundleMacos)
fs.writeFileSync(path.join(bundleDmg, 'tabby-rs.dmg'), 'final dmg')
fs.writeFileSync(path.join(bundleMacos, 'rw.intermediate.dmg'), 'intermediate dmg')
fs.writeFileSync(path.join(bundleMacos, 'tabby-rs.app.tar.gz'), 'updater')
const applicationTarget = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-rs-app-'))
const applicationInfo = path.join(applicationTarget, 'Contents', 'Info.plist')
fs.mkdirSync(path.dirname(applicationInfo), { recursive: true })
fs.writeFileSync(applicationInfo, 'application bundle')
fs.symlinkSync(applicationTarget, path.join(bundleMacos, 'Tabby RS.app'), process.platform === 'win32' ? 'junction' : 'dir')
const staged = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-rs-staged-'))
execFileSync(process.execPath, [stager], {
    cwd: root,
    env: { ...process.env, TABBY_RS_BUNDLE_ROOT: bundle, TABBY_RS_RELEASE_STAGING: staged },
    stdio: ['ignore', 'pipe', 'pipe'],
})
assert.ok(fs.existsSync(path.join(staged, 'dmg', 'tabby-rs.dmg')))
assert.ok(fs.existsSync(path.join(staged, 'macos', 'tabby-rs.app.tar.gz')))
assert.ok(fs.existsSync(path.join(staged, 'macos', 'Tabby RS.app', 'Contents', 'Info.plist')))
assert.equal(fs.existsSync(path.join(staged, 'macos', 'rw.intermediate.dmg')), false)

console.log('Tauri release artifact contract fixtures passed')
