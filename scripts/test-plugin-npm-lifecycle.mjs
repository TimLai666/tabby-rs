import assert from 'node:assert/strict'
import crypto from 'node:crypto'
import fs from 'node:fs'
import http from 'node:http'
import os from 'node:os'
import path from 'node:path'
import { execFileSync, spawn, spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const packageName = 'tabby-rs-fixture-plugin'
const versions = ['1.0.0', '1.0.1']
const fixture = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-rs-npm-lifecycle-'))
const packageDirectory = path.join(fixture, 'packages')
const tarballs = new Map()
fs.mkdirSync(packageDirectory, { recursive: true })

function commandPath (command) {
    const lookup = process.platform === 'win32' ? 'where.exe' : 'which'
    const result = spawnSync(lookup, [command], { encoding: 'utf8' })
    assert.equal(result.status, 0, `${command} must be installed on the CI runner`)
    const candidates = result.stdout.split(/\r?\n/).map(value => value.trim()).filter(Boolean)
    const candidate = process.platform === 'win32'
        ? candidates.find(value => value.toLowerCase().endsWith('.cmd')) || candidates[0]
        : candidates[0]
    assert.ok(candidate, `${command} must resolve to an executable path`)
    return candidate
}

function runNpm (npmPath, args, options = {}) {
    return execFileSync(npmPath, args, {
        ...options,
        shell: process.platform === 'win32',
    })
}

for (const version of versions) {
    const directory = path.join(fixture, `package-${version}`)
    fs.mkdirSync(directory, { recursive: true })
    fs.writeFileSync(path.join(directory, 'package.json'), `${JSON.stringify({
        name: packageName,
        version,
        main: 'index.js',
        description: 'Tabby RS system npm lifecycle fixture',
    }, null, 2)}\n`)
    fs.writeFileSync(path.join(directory, 'index.js'), `module.exports = { version: '${version}' }\n`)
    runNpm(commandPath('npm'), [
        'pack', '--ignore-scripts', '--pack-destination', packageDirectory,
    ], { cwd: directory, stdio: 'pipe' })
    const tarball = path.join(packageDirectory, `${packageName}-${version}.tgz`)
    assert.ok(fs.existsSync(tarball), `npm pack did not create ${tarball}`)
    const bytes = fs.readFileSync(tarball)
    tarballs.set(version, {
        bytes,
        shasum: crypto.createHash('sha1').update(bytes).digest('hex'),
        integrity: `sha512-${crypto.createHash('sha512').update(bytes).digest('base64')}`,
    })
}

const server = http.createServer((request, response) => {
    const pathname = decodeURIComponent(new URL(request.url, 'http://127.0.0.1').pathname)
    if (process.env.TABBY_RS_NPM_E2E_DEBUG) console.error(`registry ${request.method} ${pathname}`)
    if (pathname === `/${packageName}`) {
        const versionsDocument = Object.fromEntries(versions.map(version => {
            const tarball = tarballs.get(version)
            return [version, {
                name: packageName,
                version,
                dist: {
                    tarball: `http://127.0.0.1:${server.address().port}/${packageName}/-/${packageName}-${version}.tgz`,
                    shasum: tarball.shasum,
                    integrity: tarball.integrity,
                },
            }]
        }))
        const body = JSON.stringify({
            _id: packageName,
            name: packageName,
            'dist-tags': { latest: versions.at(-1) },
            versions: versionsDocument,
        })
        response.writeHead(200, { 'content-type': 'application/json', 'content-length': Buffer.byteLength(body) })
        response.end(body)
        return
    }
    const match = pathname.match(new RegExp(`^/${packageName}/-/${packageName}-(1\\.0\\.[01])\\.tgz$`))
    if (match) {
        const bytes = tarballs.get(match[1]).bytes
        response.writeHead(200, { 'content-type': 'application/octet-stream', 'content-length': bytes.length })
        response.end(bytes)
        return
    }
    response.writeHead(404)
    response.end()
})

await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
const npm = commandPath('npm')
const env = {
    ...process.env,
    TABBY_RS_NPM_E2E_NODE: process.execPath,
    TABBY_RS_NPM_E2E_NPM: npm,
    TABBY_RS_NPM_E2E_ROOT: path.join(fixture, 'installed-plugins'),
    TABBY_RS_NPM_E2E_PACKAGE: packageName,
    npm_config_audit: 'false',
    npm_config_fund: 'false',
    npm_config_update_notifier: 'false',
    npm_config_registry: `http://127.0.0.1:${server.address().port}/`,
    npm_config_cache: path.join(fixture, 'npm-cache'),
    TABBY_RS_NPM_E2E_DEBUG: process.env.TABBY_RS_NPM_E2E_DEBUG || '',
    npm_config_loglevel: process.env.TABBY_RS_NPM_E2E_DEBUG ? 'verbose' : 'warn',
}

try {
    const cargo = spawn('cargo', [
        'test', '--manifest-path', 'src-tauri/Cargo.toml', '--test', 'npm_lifecycle',
        'npm::tests::system_npm_plugin_lifecycle', '--', '--ignored', '--nocapture',
    ], { cwd: root, env, stdio: 'inherit' })
    const result = await new Promise((resolve, reject) => {
        cargo.once('error', reject)
        cargo.once('close', (code, signal) => resolve({ code, signal }))
    })
    assert.equal(result.signal, null, `cargo test terminated by ${result.signal}`)
    assert.equal(result.code, 0, `cargo test exited with ${result.code}`)
    console.log(`System npm lifecycle fixture passed on ${process.platform}`)
} finally {
    await new Promise(resolve => server.close(resolve))
    fs.rmSync(fixture, { recursive: true, force: true })
}
