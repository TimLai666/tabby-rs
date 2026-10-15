#!/usr/bin/env node

import { readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { createHash } from 'node:crypto'
import ts from 'typescript'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const usage = `Build the production caption painter for an isolated Windows WebView fixture.
Usage: node scripts/build-windows-caption-fixture.mjs --output <NEW-JS-FILE>

Options:
  --output <file>  Required new JavaScript file; existing files are never replaced.
  --help          Show this help.

The payload installs window.__tabbyCaptionFixture without changing the document.
paint(snapshot) paints the production component using actual WebView DPR.
read() reports the current paint token, native revision, and completion after two frames.
Exit codes: 0 success/help; 1 build or output failure; 2 invalid arguments.
Example: node scripts/build-windows-caption-fixture.mjs --output /tmp/caption-painter.js
`

async function buildPayload () {
    const sourcePath = path.join(root, 'tabby-tauri/src/components/windowsCaption.ts')
    const sourceBytes = await readFile(sourcePath)
    const source = sourceBytes.toString('utf8')
    const sourceSha256 = createHash('sha256').update(sourceBytes).digest('hex')

    const transpiled = ts.transpileModule(source, {
        compilerOptions: {
            module: ts.ModuleKind.CommonJS,
            target: ts.ScriptTarget.ES2020,
        },
        reportDiagnostics: true,
    })
    const diagnostics = transpiled.diagnostics ?? []
    if (diagnostics.length > 0) {
        const detail = diagnostics
            .map(diagnostic => ts.flattenDiagnosticMessageText(diagnostic.messageText, ' '))
            .join('; ')
        throw new Error(`Unable to compile windowsCaption.ts: ${detail}`)
    }

    const shaLiteral = JSON.stringify(sourceSha256)
    return `(function () {
    'use strict';
    var module = { exports: {} };
    var exports = module.exports;
${transpiled.outputText}
    var probeId = 'tabby-caption-paint-probe';
    var token = 0;
    var state = null;

    function paint (snapshot) {
        var container = document.getElementById(probeId);
        if (!container) {
            container = document.createElement('div');
            container.id = probeId;
            document.body.appendChild(container);
        }
        var devicePixelRatio = window.devicePixelRatio;
        exports.paintWindowsCaption(container, snapshot, {
            devicePixelRatio: devicePixelRatio,
            symbolColor: [224, 224, 224, 255],
            hoverBase: 'white',
            windows11: false,
        });
        token += 1;
        var currentToken = token;
        var rect = container.getBoundingClientRect();
        state = {
            token: currentToken,
            revision: snapshot.revision,
            ready: false,
            sourceSha256: ${shaLiteral},
            devicePixelRatio: devicePixelRatio,
            bounds: { x: rect.x, y: rect.y, width: rect.width, height: rect.height },
            declaredBounds: { x: parseFloat(container.style.left), y: parseFloat(container.style.top), width: parseFloat(container.style.width), height: parseFloat(container.style.height) },
            svgCount: container.querySelectorAll('svg').length,
            interactiveCount: container.querySelectorAll('button,[role=button],a,input,[tabindex]:not([tabindex="-1"])').length,
        };
        requestAnimationFrame(function () {
            requestAnimationFrame(function () {
                if (state !== null && state.token === currentToken) {
                    state.ready = true;
                }
            });
        });
        return { token: currentToken };
    }

    function read () {
        return state === null ? null : JSON.parse(JSON.stringify(state));
    }

    window.__tabbyCaptionFixture = {
        sourceSha256: ${shaLiteral},
        paint: paint,
        read: read,
    };
})();`
}

const args = process.argv.slice(2)
if (args.length === 1 && args[0] === '--help') {
    process.stdout.write(usage)
} else if (args.length !== 2 || args[0] !== '--output' || !args[1] || args[1].startsWith('--')) {
    process.stderr.write(`Expected --output <NEW-JS-FILE>.\n${usage}`)
    process.exitCode = 2
} else {
    try {
        const payload = await buildPayload()
        await writeFile(path.resolve(args[1]), payload, { encoding: 'utf8', flag: 'wx' })
        process.stdout.write(`${path.resolve(args[1])}\n`)
    } catch (error) {
        process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`)
        process.exitCode = 1
    }
}
