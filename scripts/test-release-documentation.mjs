import { readFile } from 'node:fs/promises';

const readme = await readFile(new URL('../README.md', import.meta.url), 'utf8');
const pluginCompatibility = await readFile(new URL('../docs/plugin-compatibility.md', import.meta.url), 'utf8');
const requiredDocuments = [
  'CONTRIBUTING.md',
  'SECURITY.md',
  'docs/install.md',
  'docs/migration.md',
  'docs/plugin-compatibility.md',
  'docs/release.md',
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
