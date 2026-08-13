import { spawnSync } from 'node:child_process'

const result = spawnSync('cargo', [
    'test',
    '--manifest-path',
    'src-tauri/Cargo.toml',
    '--lib',
    'ssh::engine_integration::runs_real_authentication_and_host_key_algorithm_matrix',
    '--',
    '--ignored',
    '--nocapture',
], {
    env: { ...process.env, TABBY_RS_SSH_INTEGRATION: '1' },
    stdio: 'inherit',
})

if (result.error) throw result.error
if (result.status !== 0) process.exit(result.status ?? 1)

console.log('SSH authentication and host-key algorithm integration passed')
