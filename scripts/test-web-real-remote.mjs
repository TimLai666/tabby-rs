import { spawnSync } from 'node:child_process'

if (process.platform === 'win32') {
    throw new Error('real Web remote smoke requires Unix OpenSSH')
}

const result = spawnSync(process.execPath, ['scripts/ci/test-web-playwright-smoke.mjs'], {
    env: { ...process.env, TABBY_WEB_REAL_OPENSSH: '1' },
    stdio: 'inherit',
})

if (result.error) throw result.error
if (result.status !== 0) process.exit(result.status ?? 1)

console.log('Web real OpenSSH and SFTP smoke passed')
