import { readFile } from 'node:fs/promises';

const readme = await readFile(new URL('../README.md', import.meta.url), 'utf8');

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
