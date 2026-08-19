import { readFile } from 'node:fs/promises';

const readme = await readFile(new URL('../README.md', import.meta.url), 'utf8');
const pluginCompatibility = await readFile(new URL('../docs/plugin-compatibility.md', import.meta.url), 'utf8');
const hacking = await readFile(new URL('../HACKING.md', import.meta.url), 'utf8');
const release = await readFile(new URL('../docs/release.md', import.meta.url), 'utf8');
const releaseAcceptance = await readFile(new URL('../docs/release-acceptance.md', import.meta.url), 'utf8');
const requiredDocuments = [
  'CONTRIBUTING.md',
  'SECURITY.md',
  'docs/install.md',
  'docs/migration.md',
  'docs/plugin-compatibility.md',
  'docs/release.md',
  'docs/release-acceptance.md',
  'docs/unsupported-signing.md',
];

for (const document of requiredDocuments) {
  await readFile(new URL(`../${document}`, import.meta.url));
}

for (const text of ['unsafe-eval', 'not a sandbox', "renderer's privileges"]) {
    if (!pluginCompatibility.includes(text)) {
        throw new Error(`docs/plugin-compatibility.md is missing runtime security guidance: ${text}`);
    }
}

const requiredReadmeText = [
  'unofficial fork',
  'TimLai666/tabby-rs/releases/latest',
  'docs/install.md',
  'docs/unsupported-signing.md',
  'SmartScreen',
  'Gatekeeper',
];

for (const text of requiredReadmeText) {
  if (!readme.includes(text)) {
    throw new Error(`README.md is missing release guidance: ${text}`);
  }
}

for (const text of [
    'Tauri desktop application',
    'yarn build:tauri:frontend',
    'cargo tauri build',
    'src-tauri/target/release/bundle/',
    '`userPluginsPath`',
    '`data/plugins`',
    '`TABBY_PLUGINS` and `tabby --debug` belong to the legacy',
]) {
    if (!hacking.includes(text)) {
        throw new Error(`HACKING.md is missing Tauri workflow guidance: ${text}`);
    }
}

if (!release.includes('`evidence_only`') || !release.includes('skips publishing') || !release.includes('ephemeral updater key') || !release.includes('evidence.invalid')) {
    throw new Error('docs/release.md is missing evidence-only workflow guidance');
}

for (const text of [
    'tabby-rs-manual-platform-acceptance',
    'sourceRevision',
    'requiredChecks',
    'side-by-side',
    'Do not use a cached DMG',
    'structured limitation',
]) {
    if (!releaseAcceptance.includes(text)) {
        throw new Error(`docs/release-acceptance.md is missing acceptance guidance: ${text}`);
    }
}

for (const keyword of ['tabby-plugin', 'tabby-builtin-plugin', 'terminus-*']) {
    if (!hacking.includes(keyword)) {
        throw new Error(`HACKING.md is missing plugin keyword guidance: ${keyword}`);
    }
}

if (!hacking.includes('legacy Electron packaging helpers')) {
    throw new Error('HACKING.md must identify legacy Electron packaging helpers');
}

for (const legacyCommand of [
    'scripts/build-windows.mjs',
    'scripts/build-linux.mjs',
    'scripts/build-macos.mjs',
]) {
    if (!hacking.includes(legacyCommand)) {
        throw new Error(`HACKING.md must retain the legacy command reference: ${legacyCommand}`);
    }
}

const forbiddenReleaseLinks = [
  'github.com/Eugeny/tabby/releases',
  'nightly.link/Eugeny/tabby',
  'packagecloud.io/eugeny/tabby',
];

for (const link of forbiddenReleaseLinks) {
  if (readme.includes(link)) {
    throw new Error(`README.md still points release traffic to upstream: ${link}`);
  }
}

console.log('release documentation contract passed');
