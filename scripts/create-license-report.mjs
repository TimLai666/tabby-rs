import assert from 'node:assert/strict'
import crypto from 'node:crypto'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const licensePath = path.resolve(process.env.TABBY_RS_LICENSE_PATH || path.join(root, 'LICENSE'))
const noticesPath = path.resolve(process.env.TABBY_RS_NOTICES_PATH || path.join(root, 'THIRD_PARTY_NOTICES.md'))
const output = path.resolve(process.env.TABBY_RS_LICENSE_REPORT || path.join(root, 'license-report.json'))
const revision = process.env.GITHUB_SHA || process.env.TABBY_RS_SOURCE_REVISION || 'local'

assert.ok(fs.statSync(licensePath).isFile(), `LICENSE does not exist: ${licensePath}`)
assert.ok(fs.statSync(noticesPath).isFile(), `third-party notices do not exist: ${noticesPath}`)
const notices = fs.readFileSync(noticesPath, 'utf8')
const tableRows = notices.split('\n')
    .filter(line => line.startsWith('| ') && !line.startsWith('| ---') && !line.startsWith('| Ecosystem ') && !line.startsWith('| Package '))
    .map(line => line.split('|').slice(1, -1).map(value => value.trim()))
const packageRows = tableRows
    .filter(row => row.length === 5 || row.length === 4)
    .map(row => row.length === 5 ? row : ['npm', ...row])
assert.ok(packageRows.length > 0, 'third-party notices contain no package rows')
const dependencies = Object.fromEntries(['npm', 'cargo'].map(ecosystem => [
    ecosystem,
    packageRows
        .filter(row => row[0] === ecosystem)
        .map(([, name, version, license, manifest]) => ({ name, version, license, manifest })),
]))
assert.ok(dependencies.npm.length > 0, 'third-party notices contain no npm package rows')
assert.ok(dependencies.cargo.length > 0, 'third-party notices contain no cargo package rows')

const hash = filePath => crypto.createHash('sha256').update(fs.readFileSync(filePath)).digest('hex')
const report = {
    schemaVersion: 1,
    sourceRevision: revision,
    license: { path: path.basename(licensePath), sha256: hash(licensePath) },
    thirdPartyNotices: {
        path: path.basename(noticesPath),
        packageRows: packageRows.length,
        dependencies,
        sha256: hash(noticesPath),
    },
    passed: true,
}
fs.mkdirSync(path.dirname(output), { recursive: true })
fs.writeFileSync(output, `${JSON.stringify(report, null, 2)}\n`)
console.log(`Created license report at ${output}`)
