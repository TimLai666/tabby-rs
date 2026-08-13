#!/usr/bin/env node

import { readFile } from 'node:fs/promises'

const workflowPath = new URL('../../.github/workflows/build.yml', import.meta.url)
const workflow = await readFile(workflowPath, 'utf8')
const releaseWorkflowPath = new URL('../../.github/workflows/release.yml', import.meta.url)
const releaseWorkflow = await readFile(releaseWorkflowPath, 'utf8')

const forbiddenPatterns = [
    [/\bsecrets\s*\./i, 'repository secrets'],
    [/SENTRY_|sentry-upload|@sentry\/cli/i, 'Sentry upload'],
    [/PACKAGECLOUD_|packagecloud-action/i, 'PackageCloud publishing'],
    [/KEYGEN_TOKEN|keygenConfig/i, 'upstream Keygen publishing'],
    [/CSC_LINK|APPLE_TEAM_ID|APPSTORE_/i, 'Apple signing or notarization'],
    [/SM_API_KEY|SM_CLIENT_CERT|digicert\/code-signing/i, 'DigiCert signing'],
    [/npm\s+publish|yarn\s+publish/i, 'npm publishing'],
]

const violations = forbiddenPatterns
    .filter(([pattern]) => pattern.test(workflow))
    .map(([, description]) => description)

if (!/^\s*pull_request\s*:/m.test(workflow)) {
    violations.push('missing pull_request trigger')
}

if (!/^\s*permissions\s*:\s*$/m.test(workflow) || !/^\s*contents\s*:\s*read\s*$/m.test(workflow)) {
    violations.push('workflow permissions are not read-only')
}

const releaseTriggers = releaseWorkflow.match(/^on:\n([\s\S]*?)^permissions:/m)?.[1] ?? ''
if (!releaseTriggers || /(^|\n)\s*pull_request\s*:/m.test(releaseTriggers)) {
    violations.push('release workflow accepts pull_request events')
}
if (!/(^|\n)\s*push:\n[\s\S]*?^\s*tags:\s*$/m.test(releaseTriggers) || /(^|\n)\s*branches\s*:/m.test(releaseTriggers)) {
    violations.push('release workflow push trigger is not tag-only')
}
if (!/(^|\n)\s*schedule:\s*$/m.test(releaseTriggers) || !/(^|\n)\s*workflow_dispatch:\s*$/m.test(releaseTriggers)) {
    violations.push('release workflow is missing schedule or manual trigger')
}

if (!/^permissions:\n\s+contents:\s+read\s*$/m.test(releaseWorkflow)) {
    violations.push('release workflow top-level permissions are not read-only')
}
if (!/^  web-gate:\n[\s\S]*?^      - name: Build web container\n/m.test(releaseWorkflow)) {
    violations.push('release workflow is missing the web gate job')
}
if (!/^  build:\n\s+name: bundle \(\$\{\{ matrix\.name \}\}\)\n\s+needs: web-gate/m.test(releaseWorkflow)) {
    violations.push('release bundle job does not depend on the web gate')
}
if (!/^\s+run: yarn audit:tauri:dependencies --output release-staging\/dependency-audit\.json\s*$/m.test(releaseWorkflow)) {
    violations.push('release workflow is missing the Tauri dependency metadata audit')
}
if (!/--dependency-audit release-staging\/dependency-audit\.json/.test(releaseWorkflow)) {
    violations.push('release gate does not consume the Tauri dependency metadata audit')
}
if (!/run: cargo fmt --manifest-path src-tauri\/Cargo\.toml -- --check/.test(releaseWorkflow)) {
    violations.push('release workflow is missing the Rust formatting check')
}
if (!/run: cargo clippy --manifest-path src-tauri\/Cargo\.toml --lib/.test(releaseWorkflow)) {
    violations.push('release workflow is missing the Rust clippy check')
}
if (!/^  publish:[\s\S]*?^    permissions:\n\s+contents:\s+write\s*$/m.test(releaseWorkflow)) {
    violations.push('release publish job does not explicitly grant contents write')
}

if (violations.length > 0) {
    console.error('Unsafe baseline workflow configuration:')
    for (const violation of violations) {
        console.error(`- ${violation}`)
    }
    process.exit(1)
}

console.log('Baseline workflow has no release, signing, telemetry, or secret side effects.')
