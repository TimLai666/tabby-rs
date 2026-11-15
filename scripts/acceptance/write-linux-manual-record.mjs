#!/usr/bin/env node
// Writes the draft linux-x64 manual acceptance record from the evidence that
// linux-release-gate-checks.sh left in <staging>/manual/linux-x64/. Platform
// checks are `passed` only when every line of their evidence file is PASS;
// features stay `not-run` unless listed in FEATURE_PASSED with evidence.
//
// Usage: node scripts/acceptance/write-linux-manual-record.mjs --staging release-staging \
//          --source-revision <sha> --environment '<json>'
import crypto from 'node:crypto'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import yaml from 'js-yaml'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
const args = process.argv.slice(2)
const arg = name => { const i = args.indexOf(name); return i === -1 ? null : args[i + 1] }
const staging = path.resolve(arg('--staging') || 'release-staging')
const revision = arg('--source-revision')
const environment = JSON.parse(arg('--environment') || '{}')
if (!revision) throw new Error('--source-revision is required')

const evidenceDir = 'manual/linux-x64'
const read = name => {
    const file = path.join(staging, evidenceDir, name)
    return fs.existsSync(file) ? fs.readFileSync(file, 'utf8') : ''
}
const results = name => read(name).split('\n').filter(line => /^(PASS|FAIL) /.test(line))
const allPassed = names => names.every(name => results(name).length > 0 && results(name).every(line => line.startsWith('PASS')))
const passLines = names => names.flatMap(name => results(name).filter(line => line.startsWith('PASS')).map(line => line.slice(5)))
const evidence = names => names.map(name => `${evidenceDir}/${name}`)

const notRunFile = 'not-run.txt'
const environmentNote = 'Ubuntu 24.04 build container under Xvfb + openbox, not a desktop session on the supported Ubuntu 22.04 host'

const platformChecks = {
    'local-shell': { files: ['local-shell.txt'] },
    'x11-or-wayland': {
        files: ['local-shell.txt', 'wayland.txt'],
        extra: ['X11: launch, keyboard input, Ctrl+C and window resize driven with xdotool under Xvfb + openbox',
            'Wayland: launch only under weston 13 headless with GDK_BACKEND=wayland; no input was driven'],
    },
    webkitgtk: {
        files: ['local-shell.txt'],
        extra: [`installed WebKitGTK: ${environment.webview || 'unknown'}`, 'complete UI started and the terminal accepted typed input'],
    },
    ssh: {
        files: ['ssh.txt'],
        extra: ['loopback OpenSSH 9.6p1 with publickey auth; known_hosts was pre-seeded, so the host-key prompt UI was not exercised',
            'reconnect = a second `tabby-rs profile loopback` invocation opening a new session, not the in-tab reconnect prompt'],
    },
    deb: { files: ['deb.txt'], extra: ['dpkg --unpack/--purge into an isolated root as a non-root user, not a system install with apt'] },
    rpm: { files: ['rpm.txt'], extra: ['rpm --install/--erase --nodeps into an isolated root via sudo, not a system install with dnf'] },
    appimage: { files: ['appimage.txt'], extra: ['no /dev/fuse in the container: launched with APPIMAGE_EXTRACT_AND_RUN=1'] },
    'side-by-side': {
        files: ['side-by-side.txt'],
        extra: ['original Tabby 1.0.235 copied from /opt/Tabby, run with --no-sandbox and HOME pointed at a scratch directory'],
    },
    'secret-service': {
        notRun: 'not exercised: the container has no unlocked Secret Service session; both the available and the unavailable path need a real desktop keyring',
    },
    serial: { notRun: 'not exercised: no serial device or pseudo-terminal fixture accepted by the native backend is attached to the container' },
}

const featurePartial = {
    'local-shell.profiles-and-pty': 'partly exercised in local-shell.txt (PTY input, resize, SIGINT, exit); profiles, output backpressure and the platform shell matrix not checked',
    'ssh.shell-session': 'partly exercised in ssh.txt (publickey login, input, PTY resize, exit, cleanup); Unicode, environment, exit status and keepalive not checked',
    'ssh.reconnect-and-restore': 'only a second CLI launch was checked; the in-tab reconnect prompt and session recovery were not',
    'release.bundle-audit': 'AppImage, deb and rpm were built and inspected; the release bundle audit reports the Ubuntu 24.04 gnutls test vectors (see bundle-audit.json)',
    'release.install-and-side-by-side': 'install, launch and uninstall in isolated roots plus a side-by-side run were checked; installer-smoke.json failed on the same gnutls finding',
    'release.performance-reports': 'benchmark reports exist under benchmarks/ but no maintainer review was recorded',
    'release.provenance-signature-and-docs': 'artifacts are signed with an ephemeral evidence-only key; real signing, checksums review and docs were not checked',
    'desktop.cli-url-portable': 'portable mode and `profile <name>` were used by the checks; URL scheme and other CLI commands not checked',
}

const record = {
    schemaVersion: 1,
    kind: 'tabby-rs-manual-platform-acceptance',
    sourceRevision: revision,
    platform: 'linux-x64',
    architecture: 'x86_64',
    target: 'x86_64-unknown-linux-gnu',
    draft: true,
    environment: { ...environment, session: environmentNote },
    checks: [],
    features: [],
    artifacts: [],
}

const notRunLines = ['# linux-x64 manual acceptance: items not run', `# source revision ${revision}`, `# ${environmentNote}`, '']
const matrix = yaml.load(fs.readFileSync(path.join(root, 'parity/platform-matrix.yaml'), 'utf8'))
const linux = matrix.platforms.find(platform => platform.id === 'linux-x64')
for (const id of linux.requiredChecks) {
    const spec = platformChecks[id]
    if (!spec) throw new Error(`no check specification for ${id}`)
    if (spec.notRun) {
        notRunLines.push(`check ${id}: ${spec.notRun}`)
        record.checks.push({ id, status: 'not-run', steps: [spec.notRun], evidence: [`${evidenceDir}/${notRunFile}`] })
        continue
    }
    const status = allPassed(spec.files) ? 'passed' : 'failed'
    record.checks.push({ id, status, steps: [...passLines(spec.files), ...(spec.extra || [])], evidence: evidence(spec.files) })
}

const features = yaml.load(fs.readFileSync(path.join(root, 'parity/features.yaml'), 'utf8')).features
for (const feature of features.filter(f => (f.platforms || []).includes('linux') && (f.tests?.manual || []).length)) {
    const partial = featurePartial[feature.id]
    const reason = partial
        ? `not accepted: ${partial}`
        : `not run: the manual flow ${feature.tests.manual.join(', ')} needs a desktop session on the supported host`
    notRunLines.push(`feature ${feature.id}: ${reason}`)
    record.features.push({ id: feature.id, status: 'not-run', steps: [reason], evidence: [`${evidenceDir}/${notRunFile}`] })
}

for (const dir of ['appimage', 'deb', 'rpm']) {
    for (const name of fs.readdirSync(path.join(staging, dir))) {
        if (!/\.(AppImage|deb|rpm)$/.test(name)) continue
        const relative = `${dir}/${name}`
        record.artifacts.push({ path: relative, sha256: crypto.createHash('sha256').update(fs.readFileSync(path.join(staging, relative))).digest('hex') })
    }
}

fs.writeFileSync(path.join(staging, evidenceDir, notRunFile), `${notRunLines.join('\n')}\n`)
fs.mkdirSync(path.join(staging, 'manual-acceptance'), { recursive: true })
const output = path.join(staging, 'manual-acceptance', 'linux-x64.json')
fs.writeFileSync(output, `${JSON.stringify(record, null, 2)}\n`)
console.log(`wrote ${output}: ${record.checks.filter(c => c.status === 'passed').length}/${record.checks.length} checks passed, ${record.features.length} features not accepted`)
