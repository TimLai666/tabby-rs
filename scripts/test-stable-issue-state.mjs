import assert from 'node:assert/strict'
import http from 'node:http'
import { spawn } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import path from 'node:path'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const script = path.join(root, 'scripts', 'assert-stable-issue-state.mjs')
const states = new Map([
    ['2', { state: 'closed', title: 'Foundation' }],
    ['3', { state: 'open', title: 'Desktop' }],
])
let requests = 0
const server = http.createServer((request, response) => {
    requests += 1
    const match = request.url?.match(/^\/repos\/example\/tabby\/issues\/(\d+)$/)
    const issue = match ? states.get(match[1]) : null
    if (!issue) {
        response.writeHead(404, { 'content-type': 'application/json' })
        response.end(JSON.stringify({ message: 'not found' }))
        return
    }
    response.writeHead(200, { 'content-type': 'application/json' })
    response.end(JSON.stringify(issue))
})

await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
const apiBase = `http://127.0.0.1:${server.address().port}`
const run = args => new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [script, ...args], { cwd: root })
    let stdout = ''
    let stderr = ''
    child.stdout.on('data', chunk => { stdout += chunk })
    child.stderr.on('data', chunk => { stderr += chunk })
    child.on('error', reject)
    child.on('close', (status, signal) => resolve({ status, signal, stdout, stderr }))
})

try {
    const blocked = await run(['--channel', 'stable', '--repo', 'example/tabby', '--issues', '2,3', '--api-base', apiBase])
    assert.equal(blocked.status, 1)
    assert.match(blocked.stderr, /#3 open Desktop/)

    states.set('3', { state: 'closed', title: 'Desktop' })
    const passed = await run(['--channel', 'stable', '--repo', 'example/tabby', '--issues', '2,3', '--api-base', apiBase])
    assert.equal(passed.status, 0)
    assert.match(passed.stdout, /passed for 2 Epic #1 child issue/)

    const beforeNightly = requests
    const nightly = await run(['--channel', 'nightly', '--repo', 'example/tabby', '--issues', '2,3', '--api-base', apiBase])
    assert.equal(nightly.status, 0)
    assert.equal(requests, beforeNightly)
} finally {
    await new Promise(resolve => server.close(resolve))
}

console.log('Stable issue state fixtures passed')
