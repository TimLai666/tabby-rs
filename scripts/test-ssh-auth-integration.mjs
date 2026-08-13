import { spawnSync } from 'node:child_process'

const result = spawnSync('cargo', [
    'test',
    '--manifest-path',
    'src-tauri/Cargo.toml',
    '--lib',
    'ssh::engine_integration',
    '--',
    '--ignored',
    '--test-threads=1',
    '--nocapture',
], {
    env: { ...process.env, TABBY_RS_SSH_INTEGRATION: '1' },
    stdio: 'inherit',
})

if (result.error) throw result.error
if (result.status !== 0) process.exit(result.status ?? 1)

console.log('SSH authentication, host-key algorithm, and connection-fault integration passed')
