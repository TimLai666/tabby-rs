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

assert.ok(Array.isArray(manifestPlatforms) && manifestPlatforms.length > 0, 'platform manifest must define platforms')
assert.ok(Array.isArray(workflowPlatforms) && workflowPlatforms.length > 0, 'release workflow must define a build matrix')

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
