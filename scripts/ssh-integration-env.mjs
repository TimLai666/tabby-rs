import { fileURLToPath } from 'node:url'

const windowsManifest = fileURLToPath(new URL('../src-tauri/windows-app-manifest.xml', import.meta.url))

export function createSshIntegrationEnv ({ platform = process.platform, env = process.env, manifestPath = windowsManifest } = {}) {
    const result = { ...env, TABBY_RS_SSH_INTEGRATION: '1' }
    if (platform === 'win32') {
        result.RUSTFLAGS = [
            env.RUSTFLAGS,
            `-C link-arg=/MANIFESTINPUT:${manifestPath}`,
            '-C link-arg=/MANIFEST:EMBED',
        ].filter(Boolean).join(' ')
    }
    return result
}
