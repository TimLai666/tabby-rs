import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(fileURLToPath(new URL('..', import.meta.url)))
const work = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-rs-license-report-test-'))
const notices = path.join(work, 'THIRD_PARTY_NOTICES.md')
const report = path.join(work, 'license-report.json')
const htmlReport = path.join(work, 'license-report.html')

execFileSync(process.execPath, [path.join(root, 'scripts/generate-third-party-notices.mjs'), '--output', notices], {
    cwd: root,
    stdio: 'ignore',
})
const noticesText = fs.readFileSync(notices, 'utf8')
assert.match(noticesText, /\| npm \|/)
assert.match(noticesText, /\| cargo \|/)

execFileSync(process.execPath, [path.join(root, 'scripts/create-license-report.mjs')], {
    cwd: root,
    env: { ...process.env, TABBY_RS_LICENSE_REPORT: report, TABBY_RS_NOTICES_PATH: notices },
    stdio: 'ignore',
})
const licenseReport = JSON.parse(fs.readFileSync(report, 'utf8'))
assert.equal(licenseReport.passed, true)
assert.ok(licenseReport.thirdPartyNotices.dependencies.npm.length > 0)
assert.ok(licenseReport.thirdPartyNotices.dependencies.cargo.length > 0)

execFileSync(process.execPath, [path.join(root, 'scripts/create-license-report.mjs')], {
    cwd: root,
    env: {
        ...process.env,
        TABBY_RS_LICENSE_REPORT: report,
        TABBY_RS_LICENSE_REPORT_HTML: htmlReport,
        TABBY_RS_NOTICES_PATH: notices,
        TABBY_RS_SOURCE_REVISION: 'fixture-<revision>',
    },
    stdio: 'ignore',
})
const html = fs.readFileSync(htmlReport, 'utf8')
assert.match(html, /<title>Tabby RS license report<\/title>/)
assert.match(html, /fixture-&lt;revision&gt;/)
assert.doesNotMatch(html, /<revision>/)

console.log('License report fixtures passed')
