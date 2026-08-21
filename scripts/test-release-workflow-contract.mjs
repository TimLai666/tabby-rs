import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const workflow = fs.readFileSync(path.join(root, '.github/workflows/release.yml'), 'utf8')

const includeEvidenceIndex = workflow.indexOf('- name: Include manual platform acceptance evidence')
const parityReportIndex = workflow.indexOf('- name: Generate parity report')
assert.ok(includeEvidenceIndex >= 0, 'release workflow must include manual acceptance evidence')
assert.ok(parityReportIndex >= 0, 'release workflow must generate a parity report')
assert.ok(includeEvidenceIndex < parityReportIndex, 'manual evidence must be staged before parity report generation')
assert.match(workflow, /compare-parity\.mjs --report-only --evidence-root release-staging/)
assert.match(workflow, /check-release-gate\.mjs --evidence-root release-staging/)
for (const command of ['test:release-gate-contract', 'test:manual-platform-acceptance', 'test:release-gate-aggregate']) {
    assert.match(workflow, new RegExp(`yarn ${command}`), `release workflow must run ${command}`)
}

console.log('Release workflow contract passed')
