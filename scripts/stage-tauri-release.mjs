import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'

const source = process.env.TABBY_RS_BUNDLE_ROOT
const destination = path.resolve(process.env.TABBY_RS_RELEASE_STAGING || 'release-staging')
assert.ok(source, 'TABBY_RS_BUNDLE_ROOT is required')
assert.ok(fs.existsSync(source), `bundle directory does not exist: ${source}`)

function shouldStage (relativePath) {
    const normalized = relativePath.split(path.sep).join('/')
    return !(normalized.startsWith('macos/rw.') && normalized.endsWith('.dmg'))
}

function copyTree (from, to, relativeDirectory = '') {
    for (const entry of fs.readdirSync(from, { withFileTypes: true })) {
        const sourcePath = path.join(from, entry.name)
        const destinationPath = path.join(to, entry.name)
        const relativePath = path.join(relativeDirectory, entry.name)
        if (entry.isDirectory()) {
            fs.mkdirSync(destinationPath, { recursive: true })
            copyTree(sourcePath, destinationPath, relativePath)
        } else if (entry.isFile() && shouldStage(relativePath)) {
            fs.copyFileSync(sourcePath, destinationPath)
        }
    }
}

fs.mkdirSync(destination, { recursive: true })
copyTree(source, destination)
console.log(`Staged Tauri bundle files from ${source} to ${destination}`)
