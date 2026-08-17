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
const evidenceOnlyConcurrency = "\${{ github.event_name == 'workflow_dispatch' && inputs.evidence_only == true }}"

assert.ok(Array.isArray(manifestPlatforms) && manifestPlatforms.length > 0, 'platform manifest must define platforms')
assert.ok(Array.isArray(workflowPlatforms) && workflowPlatforms.length > 0, 'release workflow must define a build matrix')
assert.deepEqual(workflowDispatch.inputs?.evidence_only, {
    description: 'Build and upload release evidence without publishing',
    required: true,
    default: false,
    type: 'boolean',
}, 'manual release workflow must expose an explicit evidence-only mode')
assert.equal(releaseWorkflow?.concurrency?.['cancel-in-progress'], evidenceOnlyConcurrency, 'evidence-only runs must cancel stale evidence runs without interrupting scheduled or tagged releases')

const webGateSteps = releaseWorkflow?.jobs?.['web-gate']?.steps || []
const buildSteps = releaseWorkflow?.jobs?.build?.steps || []
const aggregateSteps = releaseWorkflow?.jobs?.aggregate?.steps || []
const upstreamTagFetch = "git fetch --force --tags https://github.com/Eugeny/tabby.git '+refs/tags/*:refs/tags/*'"
function assertUpstreamTagsBeforeInstall (steps, jobName) {
    const fetchIndex = steps.findIndex(step => step.name === 'Fetch upstream tags')
    const installIndex = steps.findIndex(step => step.name === 'Install dependencies')
    assert.ok(fetchIndex >= 0, `${jobName} must fetch upstream tags`)
    assert.equal(steps[fetchIndex]?.run, upstreamTagFetch, `${jobName} must use the pinned upstream tag fetch command`)
    assert.ok(installIndex >= 0, `${jobName} must install dependencies`)
    assert.ok(fetchIndex < installIndex, `${jobName} must fetch upstream tags before dependency installation`)
}
assertUpstreamTagsBeforeInstall(webGateSteps, 'release Web gate')
assertUpstreamTagsBeforeInstall(buildSteps, 'release bundle job')
const linuxDependencies = webGateSteps.find(step => step.name === 'Install Linux build dependencies')
const realWebRemoteSmoke = webGateSteps.find(step => step.name === 'Test web real OpenSSH SFTP Telnet smoke')
const webBundleAudit = webGateSteps.find(step => step.name === 'Verify tabby-web bundle boundary')
const sharedPluginBuild = webGateSteps.find(step => step.name === 'Build shared plugin bundles')
const webContainerBuild = webGateSteps.find(step => step.name === 'Build web container')
const typingsBuild = buildSteps.find(step => step.name === 'Build TypeScript declarations')
const tauriRendererBuild = buildSteps.find(step => step.name === 'Build Tauri renderer')
const stableIssueGate = webGateSteps.find(step => step.name === 'Enforce Stable child issue gate')
const platformGate = buildSteps.find(step => step.name === 'Enforce release gate')
const aggregateGate = aggregateSteps.find(step => step.name === 'Enforce release-wide gate')
const bundleUpload = buildSteps.find(step => step.name === 'Upload bundle')
const evidenceFallback = buildSteps.find(step => step.name === 'Write incomplete evidence fallback')
const aggregateDownload = aggregateSteps.find(step => step.name === 'Download platform gates')
const aggregateFallback = aggregateSteps.find(step => step.name === 'Write incomplete aggregate evidence fallback')
const aggregateUpload = aggregateSteps.find(step => step.name === 'Upload aggregate gate')
const evidenceCondition = "github.event_name == 'workflow_dispatch' && inputs.evidence_only == true"
const evidenceContinueOnError = `\${{ ${evidenceCondition} }}`
const evidenceUploadCondition = `\${{ always() && ((${evidenceCondition}) || success()) }}`
const evidenceMissingFilesPolicy = `\${{ ${evidenceCondition} && 'warn' || 'error' }}`
assert.equal(stableIssueGate?.if, "github.event_name != 'workflow_dispatch' || inputs.evidence_only != true", 'evidence-only runs must skip the stable issue gate')
assert.match(linuxDependencies?.run || '', /openssh-server/, 'release Web gate must install OpenSSH for the real remote smoke')
assert.match(linuxDependencies?.run || '', /xvfb/, 'release Web gate must install xvfb for the browser smoke')
assert.match(linuxDependencies?.run || '', /libayatana-appindicator3-dev/, 'release Web gate must install the Linux app indicator development library')
assert.match(linuxDependencies?.run || '', /librsvg2-dev/, 'release Web gate must install the SVG development library')
assert.match(linuxDependencies?.run || '', /libwebkit2gtk-4\.1-dev/, 'release Web gate must install the WebKitGTK development library')
assert.match(linuxDependencies?.run || '', /patchelf/, 'release Web gate must install patchelf for Tauri diagnostics builds')
assert.equal(realWebRemoteSmoke?.run, 'yarn test:web-real-remote', 'release Web gate must run the real SSH/SFTP/Telnet smoke')
assert.equal(webBundleAudit?.run, 'yarn audit:web web/dist', 'release Web gate must audit the web build output')
assert.equal(sharedPluginBuild?.run, 'yarn build', 'release Web gate must build shared plugin bundles')
assert.ok(webGateSteps.indexOf(sharedPluginBuild) < webGateSteps.indexOf(webContainerBuild), 'shared plugin bundles must be built before the Web container')
assert.equal(typingsBuild?.run, 'yarn build:typings', 'release bundle job must build TypeScript declarations')
assert.ok(buildSteps.indexOf(typingsBuild) < buildSteps.indexOf(tauriRendererBuild), 'TypeScript declarations must be built before the Tauri renderer')
assert.equal(platformGate?.['continue-on-error'], evidenceContinueOnError, 'evidence-only runs must upload platform gate reports even when they fail')
assert.equal(aggregateGate?.['continue-on-error'], evidenceContinueOnError, 'evidence-only runs must upload the aggregate gate report even when it fails')
assert.equal(evidenceFallback?.if, `\${{ always() && ${evidenceCondition} }}`, 'evidence-only runs must create an explicit incomplete gate report after early failures')
assert.match(evidenceFallback?.run || '', /evidence collection stopped before release gate evaluation/, 'evidence fallback must explain why the gate is incomplete')
assert.equal(bundleUpload?.if, evidenceUploadCondition, 'evidence-only runs must upload partial platform evidence after an earlier failure')
assert.equal(bundleUpload?.with?.['if-no-files-found'], evidenceMissingFilesPolicy, 'evidence-only platform uploads must tolerate missing partial staging')
assert.equal(releaseWorkflow?.jobs?.aggregate?.if, `\${{ always() && (needs.build.result == 'success' || (${evidenceCondition})) }}`, 'evidence-only runs must aggregate even when a platform job fails')
assert.equal(aggregateDownload?.['continue-on-error'], evidenceContinueOnError, 'evidence-only aggregation must tolerate missing platform artifacts')
assert.equal(aggregateFallback?.if, `\${{ always() && ${evidenceCondition} }}`, 'evidence-only runs must create an aggregate fallback after early failures')
assert.match(aggregateFallback?.run || '', /evidence aggregation stopped before release-wide gate evaluation/, 'aggregate fallback must explain why the gate is incomplete')
assert.equal(aggregateUpload?.if, evidenceUploadCondition, 'evidence-only runs must upload the aggregate report after partial downloads')
assert.equal(aggregateUpload?.with?.['if-no-files-found'], evidenceMissingFilesPolicy, 'evidence-only aggregate uploads must tolerate missing reports')
assert.equal(releaseWorkflow?.jobs?.publish?.if, "needs.build.result == 'success' && needs.aggregate.result == 'success' && !(github.event_name == 'workflow_dispatch' && inputs.evidence_only == true)", 'evidence-only runs must never publish a release')

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
