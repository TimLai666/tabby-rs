import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { execFileSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const checker = path.join(root, 'scripts/assert-tauri-release-artifacts.mjs')

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
fs.writeFileSync(path.join(linux, 'tabby-rs.AppImage'), 'appimage')
fs.writeFileSync(path.join(linux, 'tabby-rs.AppImage.sig'), 'signature')
fs.writeFileSync(path.join(linux, 'tabby-rs.deb'), 'deb')
fs.writeFileSync(path.join(linux, 'tabby-rs.rpm'), 'rpm')
assert.equal(run(linux, 'linux', 'appimage,deb,rpm'), path.join(linux, 'tabby-rs.AppImage'))
assert.equal(run(linux, 'linux', 'appimage,deb,rpm', true), path.join(linux, 'tabby-rs.AppImage'))
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
assert.equal(run(macos, 'macos', 'dmg'), path.join(macos, 'tabby-rs.app.tar.gz'))

console.log('Tauri release artifact contract fixtures passed')
