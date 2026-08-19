import { spawnSync } from 'node:child_process'
import { createSshIntegrationEnv } from './ssh-integration-env.mjs'

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
    env: createSshIntegrationEnv(),
    stdio: 'inherit',
})

if (result.error) throw result.error
if (result.status !== 0) process.exit(result.status ?? 1)

console.log('SSH authentication, host-key algorithm, and connection-fault integration passed')
