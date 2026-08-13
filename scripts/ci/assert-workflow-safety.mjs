#!/usr/bin/env node

import { readFile } from 'node:fs/promises'

const workflowPath = new URL('../../.github/workflows/build.yml', import.meta.url)
const workflow = await readFile(workflowPath, 'utf8')
const releaseWorkflowPath = new URL('../../.github/workflows/release.yml', import.meta.url)
const releaseWorkflow = await readFile(releaseWorkflowPath, 'utf8')
const nasmInstallerScript = await readFile(new URL('./install-nasm.ps1', import.meta.url), 'utf8')

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
if (!/^  web-gate:\n[\s\S]*?^      - name: Enforce Stable child issue gate\n/m.test(releaseWorkflow)) {
    violations.push('release workflow is missing the Stable child issue gate')
}
if (!/assert-stable-issue-state\.mjs --repo \"\$GITHUB_REPOSITORY\" --issues \"2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27\"/.test(releaseWorkflow)) {
    violations.push('Stable child issue gate does not cover all Epic #1 child issues')
}
if (!/^  build:\n\s+name: bundle \(\$\{\{ matrix\.name \}\}\)\n\s+needs: web-gate/m.test(releaseWorkflow)) {
    violations.push('release bundle job does not depend on the web gate')
}
if (!/^\s+run: yarn audit:tauri:dependencies --tauri-release --output release-staging\/dependency-audit\.json\s*$/m.test(releaseWorkflow)) {
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
if (!/^      - name: Build hardened UAC helper\n        if: runner\.os == 'Windows'\n        shell: pwsh\n        run: \|[\s\S]*?msbuild 'tabby-uac\/UAC\.sln'[\s\S]*?OutDir=\$env:GITHUB_WORKSPACE\\extras\\[\s\S]*?Test-Path \"\$env:GITHUB_WORKSPACE\\extras\\UAC\.exe\"/m.test(releaseWorkflow)) {
    violations.push('release Windows bundle does not rebuild and validate the hardened UAC helper')
}
if (!/run: pwsh -NoProfile -File scripts\/ci\/install-nasm\.ps1/.test(workflow)) {
    violations.push('Windows CI does not use the verified NASM installer')
}
if (!/run: pwsh -NoProfile -File scripts\/ci\/install-nasm\.ps1/.test(releaseWorkflow)) {
    violations.push('Windows release does not use the verified NASM installer')
}
if (!/https:\/\/www\.nasm\.us\/pub\/nasm\/releasebuilds\/3\.02\/win64\/nasm-3\.02-installer-x64\.exe/.test(nasmInstallerScript)
    || !/Get-FileHash -Path \$installer -Algorithm SHA256/.test(nasmInstallerScript)
    || !/0DDB40310861EB29F4D649FEB9466779982A2D251C0DB2B9CF0D21CF591171F3/.test(nasmInstallerScript)) {
    violations.push('NASM installer is not pinned to the verified official binary')
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
