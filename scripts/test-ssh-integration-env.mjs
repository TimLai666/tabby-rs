import assert from 'node:assert/strict'
import { createSshIntegrationEnv } from './ssh-integration-env.mjs'

const unixEnv = createSshIntegrationEnv({
    platform: 'linux',
    env: { PATH: '/usr/bin' },
})
assert.equal(unixEnv.TABBY_RS_SSH_INTEGRATION, '1')
assert.equal(unixEnv.RUSTFLAGS, undefined)

const windowsEnv = createSshIntegrationEnv({
    platform: 'win32',
    env: { PATH: 'C:\\Windows\\System32', RUSTFLAGS: '-C target-cpu=native' },
    manifestPath: 'D:\\work\\windows-app-manifest.xml',
})
assert.equal(windowsEnv.TABBY_RS_SSH_INTEGRATION, '1')
assert.match(windowsEnv.RUSTFLAGS, /-C target-cpu=native/)
assert.match(windowsEnv.RUSTFLAGS, /MANIFESTINPUT:D:\\work\\windows-app-manifest\.xml/)
assert.match(windowsEnv.RUSTFLAGS, /MANIFEST:EMBED/)

console.log('SSH integration environment contract passed')
