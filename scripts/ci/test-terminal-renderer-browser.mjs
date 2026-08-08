#!/usr/bin/env node

import { spawn } from 'node:child_process'
import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import webpack from 'webpack'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
const webNodeModules = path.join(root, 'web', 'node_modules')
const processBrowser = path.join(root, 'app', 'src', 'shims', 'process.cjs')
const outputDir = await mkdtemp(path.join(os.tmpdir(), 'tabby-renderer-parity-'))
const resultPrefix = 'TABBY_RENDERER_RESULT:'

try {
    await bundleFixture(outputDir)
    await writeFile(path.join(outputDir, 'index.html'), `<!doctype html>
<html>
<head>
<meta charset="utf-8">
<style>
html, body { margin: 0; padding: 0; background: #202024; }
.fixture { width: 820px; height: 260px; position: relative; margin: 8px; }
</style>
</head>
<body>
<div id="baseline" class="fixture"></div>
<div id="adapter" class="fixture"></div>
<div id="fallback" class="fixture"></div>
<script src="fixture.js"></script>
</body>
</html>`, 'utf8')

    const mainPath = path.join(outputDir, 'main.cjs')
    await writeFile(mainPath, `const { app, BrowserWindow } = require('electron')
const path = require('node:path')

const RESULT_PREFIX = ${JSON.stringify(resultPrefix)}
app.commandLine.appendSwitch('disable-gpu-sandbox')
app.commandLine.appendSwitch('disable-software-rasterizer', 'false')

app.whenReady().then(async () => {
  const window = new BrowserWindow({
    show: false,
    width: 900,
    height: 900,
    webPreferences: {
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: false,
    },
  })

  window.webContents.on('console-message', (_event, level, message) => {
    console.error('[renderer:' + level + '] ' + message)
  })
  window.webContents.on('render-process-gone', (_event, details) => {
    console.error('Renderer process exited: ' + JSON.stringify(details))
  })

  try {
    await window.loadFile(path.join(__dirname, 'index.html'))
    const result = await window.webContents.executeJavaScript(`
      new Promise(resolve => {
        let attempts = 0
        const poll = () => {
          const test = window.__TABBY_RENDERER_TEST__
          if (test) {
            Promise.resolve(test).then(resolve, error => resolve({
              ok: false,
              checks: [],
              error: error instanceof Error ? error.stack || error.message : String(error),
            }))
            return
          }
          if (++attempts >= 200) {
            resolve({
              ok: false,
              checks: [],
              error: 'Renderer parity fixture did not initialize within 5 seconds',
            })
            return
          }
          setTimeout(poll, 25)
        }
        poll()
      })
    `)
    console.log(RESULT_PREFIX + JSON.stringify(result))
    if (!result || !result.ok) {
      process.exitCode = 1
    }
  } catch (error) {
    console.error(error)
    console.log(RESULT_PREFIX + JSON.stringify({
      ok: false,
      checks: [],
      error: error instanceof Error ? error.stack || error.message : String(error),
    }))
    process.exitCode = 1
  } finally {
    window.destroy()
    app.quit()
  }
})

app.on('window-all-closed', () => {})
`, 'utf8')

    const electron = electronBinary()
    const command = process.platform === 'linux' ? 'xvfb-run' : electron
    const args = process.platform === 'linux'
        ? ['-a', electron, '--no-sandbox', mainPath]
        : ['--no-sandbox', mainPath]

    const result = await run(command, args, {
        ...process.env,
        ELECTRON_DISABLE_SECURITY_WARNINGS: 'true',
    })
    process.stdout.write(result.stdout)
    process.stderr.write(result.stderr)

    const payload = extractPayload(result.stdout)
    if (result.code !== 0 || !payload.ok) {
        throw new Error(payload.error ?? `Terminal renderer browser fixture failed with exit code ${result.code}`)
    }
    console.log(`Terminal renderer browser parity passed: ${payload.checks.join(', ')}`)
} finally {
    await rm(outputDir, { recursive: true, force: true })
}

async function bundleFixture (outputPath) {
    const config = {
        mode: 'development',
        target: 'web',
        context: root,
        devtool: false,
        entry: path.join(root, 'scripts/ci/terminal-renderer-browser/fixture.ts'),
        output: {
            path: outputPath,
            filename: 'fixture.js',
        },
        resolve: {
            extensions: ['.ts', '.js', '.cjs'],
            modules: [
                path.join(root, 'node_modules'),
                path.join(root, 'tabby-terminal/node_modules'),
                webNodeModules,
            ],
            fallback: {
                buffer: path.join(webNodeModules, 'buffer/index.js'),
                events: path.join(webNodeModules, 'events/events.js'),
                path: path.join(webNodeModules, 'path-browserify/index.js'),
                process: processBrowser,
                stream: path.join(webNodeModules, 'stream-browserify/index.js'),
                util: path.join(webNodeModules, 'util/util.js'),
                fs: false,
                os: false,
            },
        },
        module: {
            rules: [
                {
                    test: /\.ts$/,
                    exclude: /node_modules/,
                    use: {
                        loader: 'ts-loader',
                        options: {
                            transpileOnly: true,
                            compilerOptions: {
                                module: 'esnext',
                                target: 'es2020',
                            },
                        },
                    },
                },
                {
                    test: /\.css$/,
                    use: ['style-loader', 'css-loader'],
                },
                {
                    test: /lib[\\/]xterm-addon-image-worker\.js$/i,
                    type: 'asset/source',
                },
            ],
        },
        performance: { hints: false },
    }

    await new Promise((resolve, reject) => {
        webpack(config, (error, stats) => {
            if (error) {
                reject(error)
                return
            }
            if (!stats || stats.hasErrors()) {
                reject(new Error(stats?.toString({ colors: false, errors: true, warnings: false }) ?? 'Webpack fixture build failed'))
                return
            }
            resolve()
        })
    })
}

function extractPayload (stdout) {
    const ansi = /\x1b\[[0-9;]*m/g
    for (const line of stdout.split(/\r?\n/).reverse()) {
        const clean = line.replace(ansi, '')
        const index = clean.indexOf(resultPrefix)
        if (index !== -1) {
            return JSON.parse(clean.slice(index + resultPrefix.length))
        }
    }
    return {
        ok: false,
        checks: [],
        error: 'Terminal renderer fixture did not emit a structured result',
    }
}

function electronBinary () {
    if (process.platform === 'win32') {
        return path.join(root, 'node_modules', 'electron', 'dist', 'electron.exe')
    }
    if (process.platform === 'darwin') {
        return path.join(root, 'node_modules', 'electron', 'dist', 'Electron.app', 'Contents', 'MacOS', 'Electron')
    }
    return path.join(root, 'node_modules', 'electron', 'dist', 'electron')
}

function run (command, args, env) {
    return new Promise((resolve, reject) => {
        const child = spawn(command, args, { cwd: root, env })
        let stdout = ''
        let stderr = ''
        child.stdout.on('data', chunk => { stdout += chunk })
        child.stderr.on('data', chunk => { stderr += chunk })
        child.on('error', reject)
        child.on('close', code => resolve({ code, stdout, stderr }))
    })
}
