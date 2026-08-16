import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import yaml from 'js-yaml'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const platformManifest = yaml.load(fs.readFileSync(path.join(root, 'parity/platform-matrix.yaml'), 'utf8'))
const releaseWorkflow = yaml.load(fs.readFileSync(path.join(root, '.github/workflows/release.yml'), 'utf8'))
const manifestPlatforms = platformManifest?.platforms || []
const workflowPlatforms = releaseWorkflow?.jobs?.build?.strategy?.matrix?.include || []
const workflowTriggers = releaseWorkflow?.on || releaseWorkflow?.true || {}
const workflowDispatch = workflowTriggers.workflow_dispatch || {}

assert.ok(Array.isArray(manifestPlatforms) && manifestPlatforms.length > 0, 'platform manifest must define platforms')
assert.ok(Array.isArray(workflowPlatforms) && workflowPlatforms.length > 0, 'release workflow must define a build matrix')
assert.deepEqual(workflowDispatch.inputs?.evidence_only, {
    description: 'Build and upload release evidence without publishing',
    required: true,
    default: false,
    type: 'boolean',
}, 'manual release workflow must expose an explicit evidence-only mode')

const webGateSteps = releaseWorkflow?.jobs?.['web-gate']?.steps || []
const buildSteps = releaseWorkflow?.jobs?.build?.steps || []
const aggregateSteps = releaseWorkflow?.jobs?.aggregate?.steps || []
const stableIssueGate = webGateSteps.find(step => step.name === 'Enforce Stable child issue gate')
const platformGate = buildSteps.find(step => step.name === 'Enforce release gate')
const aggregateGate = aggregateSteps.find(step => step.name === 'Enforce release-wide gate')
assert.equal(stableIssueGate?.if, "github.event_name != 'workflow_dispatch' || inputs.evidence_only != true", 'evidence-only runs must skip the stable issue gate')
assert.equal(platformGate?.['continue-on-error'], "${{ github.event_name == 'workflow_dispatch' && inputs.evidence_only == true }}", 'evidence-only runs must upload platform gate reports even when they fail')
assert.equal(aggregateGate?.['continue-on-error'], "${{ github.event_name == 'workflow_dispatch' && inputs.evidence_only == true }}", 'evidence-only runs must upload the aggregate gate report even when it fails')
assert.match(releaseWorkflow?.jobs?.publish?.if || '', /inputs\.evidence_only == true/, 'evidence-only runs must never publish a release')

const project = entries => new Map(entries.map(entry => [entry.id || entry.name, entry]))
const manifest = project(manifestPlatforms)
const workflow = project(workflowPlatforms)
assert.deepEqual([...workflow.keys()].sort(), [...manifest.keys()].sort(), 'release workflow and platform manifest must define the same platform ids')

for (const [id, platform] of manifest) {
    const build = workflow.get(id)
    assert.equal(build.os, platform.runner, `${id} runner must match release workflow os`)
    assert.equal(build.target, platform.target, `${id} target must match release workflow target`)
    const [platformName, architecture] = id.split('-').reduce((parts, segment) => {
        if (segment === 'x64' || segment === 'arm64') parts[1] = segment
        else parts[0] = parts[0] ? `${parts[0]}-${segment}` : segment
        return parts
    }, ['', ''])
    const expectedArch = architecture === 'arm64' ? 'aarch64' : 'x86_64'
    assert.equal(build.platform, platformName, `${id} platform must match its manifest id`)
    assert.equal(build.arch, expectedArch, `${id} arch must match its manifest id`)
}

console.log(`Release platform matrix contract passed for ${manifest.size} platforms`)
