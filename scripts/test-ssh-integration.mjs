import { spawnSync } from 'node:child_process'

if (process.platform === 'win32') {
    throw new Error('SSH integration fixture requires a Unix OpenSSH server')
}

const result = spawnSync('cargo', [
    'test',
    '--manifest-path',
    'src-tauri/Cargo.toml',
    '--lib',
    'ssh::sftp::integration::runs_real_ssh_shell_and_sftp_lifecycle',
    '--',
    '--ignored',
    '--nocapture',
], {
    env: { ...process.env, TABBY_RS_SSH_INTEGRATION: '1' },
    stdio: 'inherit',
})

if (result.error) throw result.error
if (result.status !== 0) process.exit(result.status ?? 1)

console.log('OpenSSH shell and SFTP integration passed')
